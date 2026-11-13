use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn service(
    state: &KernelRuntimeState,
    responses: Vec<(&'static str, u16, serde_json::Value)>,
) -> tokio::task::JoinHandle<()> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api_url = format!("http://{}", listener.local_addr().unwrap());
    let mut config = state.owned.config_projection.snapshot();
    config.relay_url = Some("wss://relay.test".into());
    config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
        kernel_id: Some(config.daemon_id.clone()),
        kernel_credential: Some("synthetic-test-credential".into()),
        kernel_public_key_thumbprint: None,
        api_url,
        email: "user@example.test".into(),
        account_id: "account".into(),
        user_id: state.kernel_owner_user_id(),
        account_slug: "account".into(),
        realm_id: "realm".into(),
        relay_url: "wss://relay.test".into(),
        issuer_id: "issuer".into(),
        client_id: None,
        client_alias: None,
        machine_id: None,
        machine_credential: None,
        cloud_session_token: None,
        cloud_session_expires_at_ms: None,
        token_expires_at_ms: None,
    });
    state.owned.config_projection.update(config);
    tokio::spawn(async move {
        for (action, status, response) in responses {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let header_end = loop {
                let mut chunk = [0; 4096];
                let n = socket.read(&mut chunk).await.unwrap();
                assert!(n > 0 && bytes.len() < 131072);
                bytes.extend_from_slice(&chunk[..n]);
                if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                    break end + 4;
                }
            };
            let headers = std::str::from_utf8(&bytes[..header_end]).unwrap();
            let length: usize = headers
                .lines()
                .find_map(|line| {
                    let (key, value) = line.split_once(':')?;
                    key.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse().unwrap())
                })
                .unwrap();
            while bytes.len() < header_end + length {
                let mut chunk = [0; 4096];
                let n = socket.read(&mut chunk).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&chunk[..n]);
            }
            let body: serde_json::Value =
                serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap();
            assert_eq!(body["request"]["action"], action);
            let response = response.to_string();
            socket.write_all(format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).as_bytes()).await.unwrap();
        }
    })
}

fn machine(state: &str) -> serde_json::Value {
    serde_json::json!({"kind":"machine","revision":1,"available_actions":[],
        "allocation_ref":"cleanup-test","state":state,"kernel_target":null,
        "access_state":"pending","access_reason":null,"purpose":"offload_job"})
}

#[tokio::test]
async fn offload_job_cleanup_retries_service_update_at_get_release_and_poll() {
    let (state, _, _, _workspace) = super::tests::fixture().await;
    let unavailable = serde_json::json!({"error":{"code":"dependency_unavailable"}});
    let server = service(
        &state,
        vec![
            ("get", 502, serde_json::Value::Null),
            ("get", 200, machine("ready")),
            ("release", 503, unavailable.clone()),
            // Re-read after an uncertain write: it may have been committed.
            ("get", 200, machine("deleting")),
            ("get", 503, unavailable),
            ("get", 200, machine("deleted")),
        ],
    )
    .await;
    let result = tokio::time::timeout(
        Duration::from_secs(15),
        state.job_compute_release_ref("cleanup-test", "v1"),
    )
    .await
    .unwrap();
    server.abort();
    let _ = server.await;
    assert!(
        result.is_ok(),
        "service updates must not lose cleanup: {result:?}"
    );
}

#[tokio::test]
async fn offload_job_cleanup_does_not_retry_authorization_denial() {
    let (state, _, _, _workspace) = super::tests::fixture().await;
    let server = service(
        &state,
        vec![(
            "get",
            403,
            serde_json::json!({"error":{"code":"authorization_denied"}}),
        )],
    )
    .await;
    let error = tokio::time::timeout(
        Duration::from_secs(2),
        state.job_compute_release_ref("cleanup-test", "v1"),
    )
    .await
    .unwrap()
    .unwrap_err();
    server.await.unwrap();
    assert_eq!(
        crate::runtime::cloud_api_client::cloud_error_code(&error),
        Some("authorization_denied")
    );
}

#[tokio::test]
async fn offload_job_completed_result_survives_unconfirmed_cleanup() {
    let (state, run, task, _workspace) = super::tests::fixture().await;
    let value = state
        .dispatch_job_tool(
            Some(&run),
            "chariox.jobs.run",
            &serde_json::json!({"command":["/bin/echo","completed"],"wait_s":5}),
            &task,
        )
        .await
        .unwrap();
    let driver = state.job_wake_driver.as_ref().unwrap();
    let mut record = driver.read(value["job_id"].as_str().unwrap()).unwrap();
    let mut result = record.result.take().unwrap();
    result.files_back = vec!["target/build.bin".into()];
    result.conflicts = vec!["edited.txt".into()];
    record.placement = JobPlacement::Remote;
    record.allocation_ref = Some("cleanup-test".into());
    record.capability_revision = Some("v1".into());
    let mut output = super::execute::Output::new(&state, &record).unwrap();
    let server = service(
        &state,
        vec![(
            "get",
            403,
            serde_json::json!({"error":{"code":"authorization_denied"}}),
        )],
    )
    .await;
    let settled = state
        .complete_remote_job(&mut record, &mut output, Ok(result.clone()), false)
        .await;
    server.await.unwrap();
    assert!(
        settled.is_ok(),
        "cleanup must not replace the execution result: {settled:?}"
    );
    let saved = driver.read(&record.job_id).unwrap();
    assert!(!saved.deleted);
    let saved = saved
        .result
        .expect("execution result must be durable before cleanup");
    assert_eq!(saved.exit_code, Some(0));
    assert_eq!(saved.files_back, result.files_back);
    assert_eq!(saved.conflicts, result.conflicts);
    record.state = "finished".into();
    driver.save(&record).unwrap();
    let server = service(&state, vec![("get", 200, machine("deleted"))]).await;
    let status = state
        .dispatch_job_tool(
            Some(&run),
            "chariox.jobs.status",
            &serde_json::json!({"job_id":record.job_id}),
            &task,
        )
        .await
        .unwrap();
    server.await.unwrap();
    assert_eq!(status["deleted"], true);
    assert_eq!(status["result"]["exit_code"], 0);
    assert_eq!(
        status["result"]["files_back"],
        serde_json::json!(["target/build.bin"])
    );
}
