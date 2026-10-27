//! Protocol 404 drill through the real websocket config request path.
use super::*;

#[tokio::test]
async fn kernel_access_config_websocket_drill() {
    let root = RuntimeTransportTempDir::new("access-config");
    let listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let mcp_listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
    let mut config = daemon_config_for_runtime_mcp_listener(&mcp_listener);
    let path = root.path().join("config.toml");
    config.user_config_path = path.clone();
    let app = Arc::new(Mutex::new(DaemonApp::bootstrap(config).unwrap()));
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let server = tokio::spawn(run_kernel_websocket_server_on_listeners_with_auth(
        app,
        listener,
        mcp_listener,
        KernelLocalAuth::Unconfigured,
        async {
            let _ = shutdown_rx.await;
        },
    ));
    let (mut socket, _) = connect_async(format!("ws://{addr}")).await.unwrap();
    let defaults = serde_json::json!({
        "grant_default_minutes": 480,
        "grant_max_minutes": 1440,
        "grant_extend_notice_minutes": 5,
        "request_timeout_minutes": 10,
    });
    let result = request(
        &mut socket,
        LocalDaemonRequest::GetUserConfig(crate::local::GetUserConfigRequest),
    )
    .await;
    assert_eq!(
        result["response"]["UserConfig"]["config"]["kernel_access"],
        defaults
    );
    for (key, value) in [
        ("grant_default_minutes", "45"),
        ("grant_max_minutes", "900"),
        ("grant_extend_notice_minutes", "7"),
        ("request_timeout_minutes", "12"),
    ] {
        let key_path = format!("kernel_access.{key}");
        let result = request(
            &mut socket,
            LocalDaemonRequest::SetUserConfigValue(crate::local::SetUserConfigValueRequest {
                path: key_path.clone(),
                value: value.into(),
            }),
        )
        .await;
        assert!(result["error"].is_null(), "{result}");
        assert_eq!(
            result["response"]["UserConfigUpdated"]["config"]["kernel_access"][key],
            value.parse::<u32>().unwrap()
        );
        let invalid = request(
            &mut socket,
            LocalDaemonRequest::SetUserConfigValue(crate::local::SetUserConfigValueRequest {
                path: key_path.clone(),
                value: "0".into(),
            }),
        )
        .await;
        assert!(!invalid["error"].is_null());
        let result = request(
            &mut socket,
            LocalDaemonRequest::GetUserConfig(crate::local::GetUserConfigRequest),
        )
        .await;
        assert_eq!(
            result["response"]["UserConfig"]["config"]["kernel_access"][key],
            value.parse::<u32>().unwrap()
        );
        let result = request(
            &mut socket,
            LocalDaemonRequest::UnsetUserConfigValue(crate::local::UnsetUserConfigValueRequest {
                path: key_path,
            }),
        )
        .await;
        assert!(result["error"].is_null(), "{result}");
        assert_eq!(
            result["response"]["UserConfigUpdated"]["config"]["kernel_access"],
            defaults
        );
    }
    socket.close(None).await.unwrap();
    shutdown_tx.send(()).unwrap();
    timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let persisted: crate::config::CharioxUserConfig =
        toml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(
        serde_json::to_value(persisted.kernel_access).unwrap(),
        defaults
    );
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn request(socket: &mut Socket, request: LocalDaemonRequest) -> Value {
    let request_id = format!("config-{:016x}", rand::random::<u64>());
    socket
        .send(Message::Text(
            serde_json::json!({
                "type": "request", "request_id": request_id, "request": request,
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
    timeout(Duration::from_secs(10), async {
        loop {
            match socket.next().await {
                Some(Ok(Message::Text(text))) => {
                    let frame: Value = serde_json::from_str(&text).unwrap();
                    if frame["request_id"] == request_id {
                        return frame;
                    }
                }
                Some(Ok(_)) => {}
                Some(Err(error)) => panic!("websocket read failed: {error}"),
                None => panic!("websocket closed before config response"),
            }
        }
    })
    .await
    .unwrap()
}
