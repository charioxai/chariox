//! Only the dedicated disposable Chromium fixture enables this test. Raw CDP
//! evaluation and HTTP target creation below are test setup, not viewer APIs.
use super::*;
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::{tungstenite::Message, WebSocketStream};

#[tokio::test]
#[ignore = "dedicated hosted production-launcher Chromium fixture only"]
async fn actual_chromium_exact_target_projection() {
    assert_eq!(
        std::env::var("CHARIOX_BROWSER_CONTROLLER_HOSTED").as_deref(),
        Ok("owned-production-browser")
    );
    let before = http("GET", "/json/list").await.as_array().unwrap().len();
    let chosen = create("Chosen App").await;
    let other = create("Other App").await; // Newest target is deliberately different.
    let mut chosen_setup = page(&chosen).await;
    let mut other_setup = page(&other).await;
    loaded(&mut chosen_setup).await;
    loaded(&mut other_setup).await;
    evaluate(
        &mut chosen_setup,
        "document.querySelector('textarea').focus(); true",
    )
    .await;
    let mut selected = target();
    selected.target_id = chosen.clone();
    let owner = BrowserController::connect(selected.clone(), Arc::new(()))
        .await
        .unwrap();
    let handle = owner.handle();
    assert_eq!(handle.target(), &selected);
    let snapshot = handle.snapshot().await.unwrap();
    assert!(snapshot.accessibility.to_string().contains("Chosen App"));
    assert!(!snapshot.accessibility.to_string().contains("Other App"));
    let input = handle
        .input(
            snapshot.reference.clone(),
            BrowserInput::Text("exact-target-effect".into()),
            Arc::new(()),
        )
        .await
        .unwrap();
    assert_eq!(input, snapshot.reference);
    assert_eq!(
        evaluate(
            &mut chosen_setup,
            "document.querySelector('textarea').value"
        )
        .await,
        "exact-target-effect"
    );
    assert_eq!(
        evaluate(&mut other_setup, "document.querySelector('textarea').value").await,
        ""
    );
    // Screencasting visibility belongs to the Room owner. This setup activates
    // only the fixture-owned target; the adapter does not steal user focus.
    call(&mut chosen_setup, "Page.bringToFront", json!({})).await;
    let mut frames = handle.frames();
    tokio::time::timeout(Duration::from_secs(5), async {
        while frames.borrow().is_none() {
            frames.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    let frame = frames.borrow().clone().unwrap();
    assert_eq!(frame.reference.tab_id, "app-tab");
    assert!(frame.jpeg.len() >= 4 && frame.css_width > 0 && frame.css_height > 0);
    owner.shutdown().await.unwrap();
    assert!(frames.borrow().is_none());
    let after = http("GET", "/json/list").await;
    for id in [&chosen, &other] {
        assert!(
            after
                .as_array()
                .unwrap()
                .iter()
                .any(|tab| tab["id"].as_str() == Some(id.as_str())),
            "controller must not close a Tab"
        );
    }
    chosen_setup.close(None).await.unwrap();
    other_setup.close(None).await.unwrap();
    // Only IDs returned by this test's target creation can be closed here.
    for id in [&chosen, &other] {
        http_bytes("GET", &format!("/json/close/{id}")).await;
    }
    assert_eq!(
        http("GET", "/json/list").await.as_array().unwrap().len(),
        before
    );
    println!("browser_controller_acceptance={{\"exactTarget\":true,\"twoTargets\":true,\"accessibility\":true,\"textInput\":true,\"screencast\":true,\"ownerClosePreservesTabs\":true,\"fixtureTargetsRemoved\":true,\"fullViewerValidated\":false}}");
}

async fn create(title: &str) -> String {
    let source = format!("data:text/html,<title>{title}</title><label>{title}<textarea autofocus></textarea></label>");
    let encoded: String = source.bytes().map(|b| format!("%{b:02X}")).collect();
    let response = http("PUT", &format!("/json/new?{encoded}")).await;
    protocol::identifier(&response["id"]).unwrap()
}
async fn http(method: &str, path: &str) -> Value {
    serde_json::from_slice(&http_bytes(method, path).await).unwrap()
}
async fn http_bytes(method: &str, path: &str) -> Vec<u8> {
    tokio::time::timeout(Duration::from_secs(3), async {
        assert!(path.len() < 8192 && !path.contains(['\r', '\n']));
        let mut stream = TcpStream::connect("127.0.0.1:9222").await.unwrap();
        stream.write_all(format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:9222\r\nConnection: close\r\nContent-Length: 0\r\n\r\n").as_bytes()).await.unwrap();
        http_body(&mut stream).await
    }).await.unwrap_or_else(|_| panic!("fixture DevTools HTTP deadline: {method} {path}"))
}

// DevTools may keep its socket open even when the fixture requests close.
// Read the declared body, not EOF. This deliberately accepts only the bounded
// Content-Length response framing emitted by the owned DevTools HTTP server.
async fn http_body(stream: &mut (impl tokio::io::AsyncRead + Unpin)) -> Vec<u8> {
    let mut bytes = Vec::new();
    let end = loop {
        if let Some(end) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
            break end;
        }
        assert!(bytes.len() < 16384, "fixture HTTP header limit");
        let mut next = [0; 1024];
        let n = stream.read(&mut next).await.unwrap();
        assert!(n > 0, "fixture HTTP truncated header");
        bytes.extend_from_slice(&next[..n]);
    };
    assert!(end <= 16384);
    let head = std::str::from_utf8(&bytes[..end]).unwrap();
    assert!(
        head.starts_with("HTTP/1.1 200"),
        "fixture HTTP status: {head}"
    );
    let mut length = None;
    for line in head.split("\r\n").skip(1) {
        let (name, value) = line.split_once(':').unwrap();
        assert!(!name.eq_ignore_ascii_case("transfer-encoding"));
        if name.eq_ignore_ascii_case("content-length") {
            assert!(length.is_none());
            length = Some(value.trim().parse::<usize>().unwrap());
        }
    }
    let length = length.expect("fixture HTTP Content-Length required");
    assert!(length <= 1024 * 1024);
    let mut body = bytes.split_off(end + 4);
    assert!(body.len() <= length);
    let received = body.len();
    body.resize(length, 0);
    stream.read_exact(&mut body[received..]).await.unwrap();
    body
}

#[tokio::test]
async fn devtools_http_body_completes_without_socket_close() {
    let (mut client, mut server) = tokio::io::duplex(1024);
    server
        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n[]")
        .await
        .unwrap();
    let body = tokio::time::timeout(Duration::from_secs(1), http_body(&mut client))
        .await
        .unwrap();
    assert_eq!(body, b"[]");
    // Server remains open throughout; completion cannot depend on EOF.
    server.write_all(b"still open").await.unwrap();
}
async fn page(id: &str) -> WebSocketStream<TcpStream> {
    let stream = TcpStream::connect("127.0.0.1:9222").await.unwrap();
    let config = WebSocketConfig::default()
        .max_frame_size(Some(MAX_WIRE_BYTES))
        .max_message_size(Some(MAX_WIRE_BYTES));
    client_async_with_config(
        format!("ws://127.0.0.1:9222/devtools/page/{id}"),
        stream,
        Some(config),
    )
    .await
    .unwrap()
    .0
}
async fn loaded(socket: &mut WebSocketStream<TcpStream>) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if evaluate(
                socket,
                "document.readyState === 'complete' && !!document.querySelector('textarea')",
            )
            .await
                == true
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
}
async fn evaluate(socket: &mut WebSocketStream<TcpStream>, expression: &str) -> Value {
    call(
        socket,
        "Runtime.evaluate",
        json!({"expression":expression,"returnByValue":true}),
    )
    .await["result"]["value"]
        .clone()
}
async fn call(socket: &mut WebSocketStream<TcpStream>, method: &str, params: Value) -> Value {
    tokio::time::timeout(Duration::from_secs(3), async {
        socket
            .send(Message::Text(
                json!({"id":1,"method":method,"params":params})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        for _ in 0..128 {
            let next = socket.next().await.unwrap().unwrap();
            let Message::Text(text) = next else { continue };
            let value: Value = serde_json::from_str(&text).unwrap();
            if value["id"] == 1 {
                assert!(value.get("error").is_none(), "fixture CDP command failed");
                return value["result"].clone();
            }
        }
        panic!("fixture CDP response bound exceeded")
    })
    .await
    .unwrap()
}
