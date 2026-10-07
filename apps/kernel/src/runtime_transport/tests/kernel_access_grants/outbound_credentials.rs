//! MP-08 / MP-10 / MP-11: actual Unix delivery, initial/update/replay and owner control.
use super::*;
pub(super) const CANARY: &str = "MP-11-worker-admission-test-only";

async fn snapshot<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    socket: &mut WebSocketStream<S>,
    owner: bool,
) -> Value {
    timeout(Duration::from_secs(10), async {
        loop {
            match socket.next().await {
                Some(Ok(Message::Text(text))) => {
                    let value: Value = serde_json::from_str(&text).unwrap();
                    if !owner {
                        assert!(
                            !text.contains(CANARY),
                            "MP-11 credential escaped external delivery"
                        );
                    }
                    if value["event"]["event"] == "session_snapshot" {
                        assert_eq!(
                            text.contains(CANARY),
                            owner,
                            "MP-11 credential visibility differs from caller authority"
                        );
                        assert_eq!(value["event"]["session"]["id"], "credential-session");
                        let agents = value["event"]["session"]["agents"].as_array().unwrap();
                        assert!(agents
                            .iter()
                            .any(|a| a["remote_execution"]["worker_kernel_id"] == "worker"));
                        return value;
                    }
                }
                Some(Ok(Message::Ping(data))) => {
                    socket.send(Message::Pong(data)).await.unwrap();
                }
                other => panic!("MP-11 expected snapshot: {other:?}"),
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
    assert!(response(&mut socket, "sub").await["error"].is_null());
    let initial = snapshot(&mut socket, false).await;
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
    assert!(response(&mut socket, "request").await["error"].is_null());
    let updated = snapshot(&mut socket, false).await;
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
    // Replay is enqueued before its ack but the event lane drains after priority.
    assert!(response(&mut socket, "sub").await["error"].is_null());
    let replay = snapshot(&mut socket, false).await;
    assert_eq!(
        replay["event_id"], updated["event_id"],
        "MP-11 must exercise retained replay"
    );
}

#[tokio::test]
async fn kernel_access_subscription_initial_updated_replay_hide_worker_credentials() {
    let mut kernel = Kernel::start().await;
    let mut holder = Client::start(&kernel.root);
    grant(&mut kernel, &mut holder).await;
    let owner = kernel
        .request(serde_json::json!({"GetSessionState":{"session_id":"credential-session"}}))
        .await;
    assert!(owner["error"].is_null(), "MP-11 owner read must succeed");
    assert_eq!(owner["response"]["SessionState"]["session"]["id"], "credential-session");
    assert!(
        owner.to_string().contains(CANARY),
        "MP-11 owner credential control"
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
    assert!(response(&mut kernel.tcp, "owner-sub").await["error"].is_null());
    snapshot(&mut kernel.tcp, true).await;
    holder.command("credential-subscribe");
    assert_eq!(holder.result()["credential_subscription"], true);
    let owner = kernel
        .request(serde_json::json!({"GetSessionState":{"session_id":"credential-session"}}))
        .await;
    assert!(
        owner.to_string().contains(CANARY),
        "MP-11 projection must not mutate stored credentials"
    );
}
