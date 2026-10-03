//! Protocol 403 over live websockets: one critical approval is a popup on two
//! terminals, one attached to its session and one in the waiting room. The
//! first correct answer closes it on both, a later answer is told it was
//! already answered, and a connection that may not submit a passkey never
//! gets the popup.
use std::collections::VecDeque;

use super::*;

const PASSKEY: &str = "correct horse battery";

struct Terminal {
    socket: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    events: VecDeque<Value>,
}

impl Terminal {
    async fn connect(kernel: &ClassAuditKernel, authorization: &str) -> Self {
        let mut request = format!("ws://{}", kernel.addr)
            .into_client_request()
            .expect("request should build");
        request.headers_mut().insert(
            AUTHORIZATION,
            HeaderValue::from_str(authorization).expect("header should be valid"),
        );
        let (socket, _) = connect_async(request)
            .await
            .expect("the connection should be accepted");
        Self {
            socket,
            events: VecDeque::new(),
        }
    }

    async fn send(&mut self, frame: Value) {
        self.socket
            .send(Message::Text(frame.to_string().into()))
            .await
            .expect("frame should send");
    }

    /// The next frame; events are kept for `next_event`.
    async fn frame(&mut self) -> Value {
        let frame = timeout(Duration::from_secs(10), async {
            loop {
                match self.socket.next().await {
                    Some(Ok(Message::Text(text))) => {
                        break serde_json::from_str::<Value>(&text).expect("frame is JSON")
                    }
                    Some(Ok(_)) => continue,
                    Some(Err(error)) => panic!("websocket read failed: {error}"),
                    None => panic!("websocket closed"),
                }
            }
        })
        .await
        .expect("a frame should arrive");
        if frame["type"] == "event" {
            self.events.push_back(frame["event"].clone());
        }
        frame
    }

    async fn response(&mut self, request_id: &str) -> Value {
        loop {
            let frame = self.frame().await;
            if frame["type"] == "response" && frame["request_id"] == request_id {
                return frame;
            }
        }
    }

    async fn request(&mut self, request: LocalDaemonRequest) -> Value {
        let request_id = format!("request-{:016x}", rand::random::<u64>());
        self.send(serde_json::json!({
            "type": "request",
            "request_id": request_id,
            "request": request,
        }))
        .await;
        self.response(&request_id).await
    }

    async fn subscribe(&mut self, session_id: &str, attachment_id: &str, scope: Option<&str>) {
        let request_id = format!("subscribe-{:016x}", rand::random::<u64>());
        self.send(serde_json::json!({
            "type": "subscribe",
            "request_id": request_id,
            "session_id": session_id,
            "attachment_id": attachment_id,
            "subscription_scope": scope,
        }))
        .await;
        let response = self.response(&request_id).await;
        assert_eq!(response["response"]["ok"], true, "{response}");
    }

    /// The next `passkey_prompts_changed` event's prompts.
    async fn prompts(&mut self) -> Vec<Value> {
        loop {
            while let Some(event) = self.events.pop_front() {
                if event["event"] == "passkey_prompts_changed" {
                    return event["prompts"].as_array().cloned().unwrap_or_default();
                }
            }
            self.frame().await;
        }
    }

    /// Answers the kernel's critical approval; the error message on refusal.
    async fn answer(
        &mut self,
        kernel: &ClassAuditKernel,
        choice: &str,
        passkey: Option<&str>,
    ) -> Option<String> {
        let response = self
            .request(LocalDaemonRequest::RespondToInteraction(
                crate::local::RespondToInteractionRequest {
                    session_id: kernel.session_id.clone(),
                    interaction_id: kernel.interaction_id.clone(),
                    choice_id: choice.into(),
                    custom_reply: None,
                    passkey: passkey.map(crate::local::ApprovalPasskey::new),
                    passkey_remember_minutes: None,
                },
            ))
            .await;
        response["error"]["message"].as_str().map(str::to_owned)
    }
}

#[cfg(unix)]
#[tokio::test]
async fn one_passkey_prompt_reaches_every_terminal_and_the_first_answer_closes_it() {
    let root = RuntimeTransportTempDir::new("passkey-popups");
    let vault = root.path().join("vault.json");
    crate::secret::create_chariox_encrypted_vault_for_test(&vault, PASSKEY)
        .expect("test vault should be created");
    let auth = Arc::new(local_auth::LocalTokenAuth::new(
        local_auth::generate_kernel_local_auth_token(),
    ));
    let kernel = ClassAuditKernel::start(
        &root,
        &vault,
        KernelLocalAuth::LocalToken(Arc::clone(&auth)),
    )
    .await;
    let token = format!("Bearer {}", auth.token());

    // Terminal one is attached to the prompt's session.
    let mut attached = Terminal::connect(&kernel, &token).await;
    let attach = attached
        .request(LocalDaemonRequest::AttachToSession(
            crate::local::AttachToSessionRequest {
                session_id: kernel.session_id.clone(),
                client_id: "popup-terminal-1".into(),
                capability_level: crate::attachment::ClientCapabilityLevel::FullTerminal,
            },
        ))
        .await;
    let attachment_id = attach["response"]["SessionAttached"]["attachment"]["id"]
        .as_str()
        .unwrap_or_else(|| panic!("attachment expected: {attach}"))
        .to_owned();
    attached
        .subscribe(&kernel.session_id, &attachment_id, None)
        .await;
    // Terminal two is not attached to any session: it is in the waiting room.
    let mut waiting = Terminal::connect(&kernel, &token).await;
    waiting
        .subscribe(
            WAITING_ROOM_INVENTORY_SENTINEL_ID,
            WAITING_ROOM_INVENTORY_SENTINEL_ID,
            Some(WAITING_ROOM_INVENTORY_SUBSCRIPTION_SCOPE),
        )
        .await;

    for terminal in [&mut attached, &mut waiting] {
        let prompts = terminal.prompts().await;
        let [prompt] = prompts.as_slice() else {
            panic!("one popup expected: {prompts:?}");
        };
        assert_eq!(prompt["kind"], "critical_approval");
        assert_eq!(prompt["session_id"], kernel.session_id.as_str());
        assert_eq!(prompt["interaction_id"], kernel.interaction_id.as_str());
        assert_eq!(prompt["approve_choice_id"], "approve");
        assert_eq!(prompt["refuse_choice_id"], "deny");
    }

    // A wrong passkey from the attached terminal leaves the popup open.
    let rejected = attached
        .answer(&kernel, "approve", Some("guess"))
        .await
        .expect("a wrong passkey is refused");
    assert!(rejected.contains("PASSKEY_REJECTED"), "{rejected}");
    // The waiting-room terminal answers first, and both popups close: the
    // next set either terminal gets is empty.
    assert_eq!(
        waiting.answer(&kernel, "approve", Some(PASSKEY)).await,
        None
    );
    assert!(waiting.prompts().await.is_empty());
    assert!(attached.prompts().await.is_empty());
    // The attached terminal's later answer was already answered.
    for (choice, passkey) in [("approve", Some(PASSKEY)), ("deny", None)] {
        let late = attached
            .answer(&kernel, choice, passkey)
            .await
            .expect("a later answer is refused");
        assert!(late.contains("PASSKEY_ALREADY_ANSWERED"), "{late}");
    }
    assert_eq!(
        kernel.audits(),
        [
            ("rejected".to_string(), serde_json::json!("terminal")),
            ("verified".to_string(), serde_json::json!("terminal")),
        ]
    );
    let _ = attached.socket.close(None).await;
    let _ = waiting.socket.close(None).await;
    kernel.stop().await;

    // A host connection never gets the popup, though one is pending.
    let host = ClassAuditKernel::start(
        &root,
        &vault,
        KernelLocalAuth::HostToken(Arc::<str>::from("kernel-local-auth-sentinel")),
    )
    .await;
    let mut controller = Terminal::connect(&host, "Bearer kernel-local-auth-sentinel").await;
    controller
        .subscribe(
            WAITING_ROOM_INVENTORY_SENTINEL_ID,
            WAITING_ROOM_INVENTORY_SENTINEL_ID,
            Some(WAITING_ROOM_INVENTORY_SUBSCRIPTION_SCOPE),
        )
        .await;
    let watched_until = TokioInstant::now() + Duration::from_millis(1_500);
    while TokioInstant::now() < watched_until {
        if timeout(Duration::from_millis(250), controller.frame())
            .await
            .is_err()
        {
            continue;
        }
    }
    assert!(controller
        .events
        .iter()
        .any(|event| event["event"] == "heartbeat"));
    assert!(controller
        .events
        .iter()
        .all(|event| event["event"] != "passkey_prompts_changed"));
    let _ = controller.socket.close(None).await;
    host.stop().await;
}
