//! MP-08 / MP-10 / MP-11: actual Unix delivery, initial/update/replay and owner control.
use super::*;
pub(super) const CANARY: &str = "MP-11-worker-admission-test-only";

// MP-08 / MP-11: real router -> executor -> mapper -> Unix socket delivery,
// not an already-mapped frame injected into the outbound policy.
#[tokio::test]
async fn kernel_access_executor_errors_keep_codes_and_retryability_over_unix() {
    let mut kernel = Kernel::start().await;
    let mut holder = Client::start(&kernel.root);
    grant(&mut kernel, &mut holder).await;
    let cases = [
        (
            serde_json::json!({"GetSessionState":{"session_id":CANARY}}),
            "session_not_found",
            false,
        ),
        (
            serde_json::json!({"GetWorkspaceFileContent":{
                "workspace_id":"workspace", "worktree_id":kernel.root.join("worktree"), "path":CANARY
            }}),
            "local_transport_error",
            true,
        ),
    ];
    for (request, code, retryable) in cases {
        // Prove the normal executor really emitted this diagnostic and semantics.
        let owner = kernel.request(request.clone()).await;
        assert!(owner["error"]["message"].as_str().unwrap().contains(CANARY));
        assert_eq!(owner["error"]["code"], code);
        assert_eq!(owner["error"]["retryable"], retryable);
        let delivered = holder.request(request);
        assert_eq!(delivered["error"]["code"], code);
        assert_eq!(delivered["error"]["retryable"], retryable);
        assert!(!delivered["error"]["message"]
            .as_str()
            .unwrap()
            .contains(CANARY));
    }
}

async fn snapshot<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    socket: &mut WebSocketStream<S>,
    reply_id: &str,
    replay_id: Option<u64>,
) -> Value {
    timeout(Duration::from_secs(10), async {
        let mut acknowledged = false;
        let mut snapshot = None;
        loop {
            match socket.next().await {
                Some(Ok(Message::Text(text))) => {
                    let value: Value = serde_json::from_str(&text).unwrap();
                    assert!(
                        !text.contains(CANARY),
                        "MP-11 credential escaped client delivery"
                    );
                    if value["type"] == "response" && value["request_id"] == reply_id {
                        assert!(
                            value["error"].is_null(),
                            "MP-11 subscription operation must succeed"
                        );
                        acknowledged = true;
                    }
                    if value["event"]["event"] == "session_snapshot" {
                        assert_eq!(value["event"]["session"]["id"], "credential-session");
                        let agents = value["event"]["session"]["agents"].as_array().unwrap();
                        assert!(agents
                            .iter()
                            .any(|a| a["remote_execution"]["worker_kernel_id"] == "worker"));
                        if replay_id.is_none_or(|id| value["event_id"].as_u64() == Some(id)) {
                            snapshot = Some(value);
                        }
                    }
                }
                Some(Ok(Message::Ping(data))) => {
                    socket.send(Message::Pong(data)).await.unwrap();
                }
                other => panic!("MP-11 expected snapshot: {other:?}"),
            }
            // Watcher events and command replies may arrive in either order.
            if acknowledged {
                if let Some(snapshot) = snapshot {
                    return snapshot;
                }
            }
        }
    })
    .await
    .unwrap()
}

pub(super) async fn check_subscription(root: &str) {
    let (mut socket, _) = client_async(
        "ws://localhost/kernel",
        tokio::net::UnixStream::connect(Path::new(root).join("run/k.sock"))
            .await
            .unwrap(),
    )
    .await
    .unwrap();
    socket.send(Message::Text(frame(serde_json::json!({"AttachToSession":{
        "session_id":"credential-session", "client_id":"secret-subscription", "capability_level":"FullTerminal"
    }})).to_string().into())).await.unwrap();
    let attached = response(&mut socket, "request").await;
    assert!(attached["error"].is_null());
    let id = attached["response"]["SessionAttached"]["attachment"]["id"]
        .as_str()
        .unwrap();
    let subscribe = |cursor: Option<u64>| {
        serde_json::json!({"type":"subscribe", "request_id":"sub",
        "session_id":"credential-session", "attachment_id":id, "resume_from_event_id":cursor})
    };
    socket
        .send(Message::Text(subscribe(None).to_string().into()))
        .await
        .unwrap();
    let initial = snapshot(&mut socket, "sub", None).await;
    let cursor = initial["event_id"].as_u64().unwrap() - 1;
    // Change agent structure so the watcher emits a new complete snapshot.
    socket
        .send(Message::Text(
            frame(serde_json::json!({"SpawnAgent":{
                "session_id":"credential-session", "provider":"codex", "model":"default"
            }}))
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
    let updated = snapshot(&mut socket, "request", None).await;
    socket
        .send(Message::Text(
            serde_json::json!({"type":"unsubscribe","request_id":"unsub"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    assert!(response(&mut socket, "unsub").await["error"].is_null());
    socket
        .send(Message::Text(subscribe(Some(cursor)).to_string().into()))
        .await
        .unwrap();
    let replay = snapshot(&mut socket, "sub", updated["event_id"].as_u64()).await;
    assert_eq!(
        replay["event_id"], updated["event_id"],
        "MP-11 must exercise retained replay"
    );
    eprintln!("MP-11 Unix initial/update/retained replay delivered without credentials");
}

#[tokio::test]
async fn kernel_access_subscription_initial_updated_replay_hide_worker_credentials() {
    let mut kernel = Kernel::start_with_credentials().await;
    let mut holder = Client::start(&kernel.root);
    grant(&mut kernel, &mut holder).await;
    kernel.control("credential-check").await;
    assert_eq!(
        std::fs::read_to_string(kernel.root.join("credential-present")).unwrap(),
        "true",
        "MP-11 stored credential precondition"
    );
    let owner = kernel
        .request(serde_json::json!({"GetSessionState":{"session_id":"credential-session"}}))
        .await;
    assert!(owner["error"].is_null(), "MP-11 owner read must succeed");
    assert_eq!(
        owner["response"]["SessionState"]["session"]["id"],
        "credential-session"
    );
    assert!(
        !owner.to_string().contains(CANARY),
        "MP-11 existing owner wire credential protection"
    );
    let attached = kernel.request(serde_json::json!({"AttachToSession":{
        "session_id":"credential-session", "client_id":"owner-control", "capability_level":"FullTerminal"
    }})).await;
    let id = attached["response"]["SessionAttached"]["attachment"]["id"]
        .as_str()
        .unwrap();
    kernel
        .tcp
        .send(Message::Text(
            serde_json::json!({"type":"subscribe","request_id":"owner-sub",
        "session_id":"credential-session", "attachment_id":id})
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
    snapshot(&mut kernel.tcp, "owner-sub", None).await;
    holder.command("credential-subscribe");
    assert_eq!(holder.result()["credential_subscription"], true);
    let owner = kernel
        .request(serde_json::json!({"GetSessionState":{"session_id":"credential-session"}}))
        .await;
    assert!(!owner.to_string().contains(CANARY));
    kernel.control("credential-check").await;
    assert_eq!(
        std::fs::read_to_string(kernel.root.join("credential-present")).unwrap(),
        "true",
        "MP-11 projection must not mutate stored credentials"
    );
}
