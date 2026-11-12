use super::*;
use crate::local::{ControlPlaneRequest, JobNeeds, KernelConnectionClass};
use crate::runtime::{command::KernelCaller, kernel_gateway::Origin};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn offload_job_cancelled_during_owner_approval_never_posts_acquire() {
    let (state, run, task, _workspace) = super::tests::fixture().await;
    let job = state
        .dispatch_job_tool(
            Some(&run),
            "chariox.jobs.run",
            &serde_json::json!({
                "command":["/bin/sleep","30"], "wait_s":0
            }),
            &task,
        )
        .await
        .unwrap();
    let id = job["job_id"].as_str().unwrap().to_owned();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let actions = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let observed = actions.clone();
    let api_url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        loop {
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
            let action = body["request"]["action"].as_str().unwrap().to_owned();
            observed.lock().unwrap().push(action.clone());
            let response = if action == "capabilities" {
                serde_json::json!({"kind":"capabilities","machine_allocation":{
                    "revision":"v1","actions":["acquire_job"],"offer":null,"job_default_class_id":"build",
                    "job_offers":[{"class_id":"build","vcpu":4,"usable_memory_mib":7000,"disk_gib":40,
                    "rate_per_minute":{"amount_micros":600,"currency":"USD"}}]}})
            } else {
                serde_json::json!({"kind":"machine","revision":1,"available_actions":[],
                    "allocation_ref":"synthetic","state":"deleted","kernel_target":null,
                    "access_state":"pending","access_reason":null,"purpose":"offload_job"})
            }.to_string();
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",response.len(),response).as_bytes()).await.unwrap();
        }
    });
    let mut config = state.owned.config_projection.snapshot();
    config.relay_url = Some("wss://relay.test".into());
    config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
        kernel_id: Some(config.daemon_id.clone()),
        kernel_credential: Some("synthetic-test-credential".into()),
        kernel_public_key_thumbprint: None,
        api_url,
        email: "user@example.test".into(),
        account_id: "account".into(),
        user_id: task.owner_user_id.clone(),
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
    let acquiring_state = state.clone();
    let acquiring_id = id.clone();
    let acquire = tokio::spawn(async move {
        acquiring_state
            .job_control_plane(
                &Origin {
                    grant_id: format!("job:{acquiring_id}"),
                    harness: "test".into(),
                },
                &ControlPlaneRequest::AcquireJob {
                    capability_revision: "v1".into(),
                    request_id: "test".into(),
                    job_ref: acquiring_id,
                    needs: JobNeeds::default(),
                    timeout_s: 60,
                },
                "test",
            )
            .await
    });
    let card = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let session = state
                .owned
                .session_store
                .get_session(&task.room_id)
                .unwrap();
            if let Some(card) = session.active_interactions().first() {
                break card.id().to_owned();
            }
            if acquire.is_finished() {
                panic!(
                    "acquisition ended before the owner decision: {:?}",
                    actions.lock().unwrap()
                );
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let caller = KernelCaller {
        user_id: Some(task.owner_user_id.clone()),
        connection_class: Some(KernelConnectionClass::Terminal),
        ..KernelCaller::default()
    };
    assert!(state
        .cancel_session_job(&task.room_id, &caller, &id)
        .unwrap());
    state
        .resolve_terminal_runtime_interaction(
            &task.room_id,
            &card,
            "approve",
            None,
            Some(&task.owner_user_id),
        )
        .await
        .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(5), acquire)
        .await
        .unwrap()
        .unwrap();
    server.abort();
    let _ = server.await;
    assert!(
        result.is_err(),
        "a cancelled job cannot acquire: {result:?}"
    );
    assert_eq!(*actions.lock().unwrap(), vec!["capabilities"]);
}
