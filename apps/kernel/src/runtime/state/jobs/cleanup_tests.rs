use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn serve_profile(state: &KernelRuntimeState) -> tokio::net::TcpListener {
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
        kernel_unlinked: false,
        kernel_removal_pending: false,
        token_expires_at_ms: None,
    });
    state.owned.config_projection.update(config);
    listener
}

/// Accepts one request and returns its parsed control-plane body.
async fn read_request(socket: &mut tokio::net::TcpStream) -> serde_json::Value {
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
    serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap()
}

async fn respond(socket: &mut tokio::net::TcpStream, status: u16, response: &serde_json::Value) {
    let response = response.to_string();
    socket.write_all(format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).as_bytes()).await.unwrap();
}

pub(super) async fn service(
    state: &KernelRuntimeState,
    responses: Vec<(&'static str, u16, serde_json::Value)>,
) -> tokio::task::JoinHandle<()> {
    let listener = serve_profile(state).await;
    tokio::spawn(async move {
        for (action, status, response) in responses {
            tokio::time::timeout(
                SCRIPTED_REQUEST_DEADLINE,
                serve_one(&listener, action, status, response),
            )
            .await
            .unwrap_or_else(|_| {
                panic!("the kernel never made the scripted `{action}` service request")
            });
        }
    })
}

/// Answers every request by its action, in any order, until the test ends it.
pub(super) async fn service_by_action(
    state: &KernelRuntimeState,
    answer: impl Fn(&serde_json::Value) -> (u16, serde_json::Value) + Send + 'static,
) -> tokio::task::JoinHandle<()> {
    let listener = serve_profile(state).await;
    tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let body = read_request(&mut socket).await;
            let (status, response) = answer(&body["request"]);
            respond(&mut socket, status, &response).await;
        }
    })
}

/// The longest a test waits for the kernel's next request to the scripted
/// service, or for a job tool to answer: what never comes ends the test with
/// its name instead of leaving the test run waiting.
pub(super) const SCRIPTED_REQUEST_DEADLINE: Duration = Duration::from_secs(60);

async fn serve_one(
    listener: &tokio::net::TcpListener,
    action: &'static str,
    status: u16,
    response: serde_json::Value,
) {
    let (mut socket, _) = listener.accept().await.unwrap();
    let body = read_request(&mut socket).await;
    assert_eq!(body["request"]["action"], action);
    respond(&mut socket, status, &response).await;
}

pub(super) fn machine(state: &str) -> serde_json::Value {
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
    let (state, run, task, _workspace) = super::tests::remote_fixture().await;
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

async fn refused_job_waits_locally(status: u16, code: &'static str, sentence: &str) {
    let value = job_refused_by_the_service(status, code, |state, task| {
        state
            .set_remote_job_policy(
                &task.room_id,
                Some(&task.agent_id),
                false,
                &task.owner_user_id,
                Some(crate::local::OffloadPolicy::AlwaysHeavy),
            )
            .unwrap();
    })
    .await;
    assert_eq!(value["placement"], "local", "{value}");
    assert_eq!(value["result"]["exit_code"], 0, "{value}");
    let tail = value["result"]["output_tail"].as_str().unwrap();
    assert!(
        tail.contains("ran-here") && tail.contains(sentence),
        "{code}: {tail}"
    );
}

/// Runs `echo ran-here` under the policy `set_policy` writes against a service that refuses every machine.
async fn job_refused_by_the_service(
    status: u16,
    code: &'static str,
    set_policy: impl FnOnce(&KernelRuntimeState, &AgentTaskExecution),
) -> serde_json::Value {
    let mut config = crate::config::DaemonConfig::for_tests();
    config.remote_jobs = true;
    config.user_config.agents.offload_without_asking = Some(true);
    let (state, run, task, _workspace) = super::tests::fixture_with(config).await;
    set_policy(&state, &task);
    let offer = serde_json::json!({"kind":"capabilities","machine_allocation":{
        "revision":"v1","actions":["acquire_job"],"offer":null,"job_default_class_id":"build",
        "job_offers":[{"class_id":"build","vcpu":4,"usable_memory_mib":7000,"disk_gib":40,
        "rate_per_minute":{"amount_micros":600,"currency":"USD"}}]}});
    let server = service_by_action(&state, move |request| {
        match request["action"].as_str().unwrap() {
            "capabilities" => (200, offer.clone()),
            "acquire_job" => (
                status,
                serde_json::json!({"error":{"code":code,"message":"The service said no"}}),
            ),
            "cancel_job" => (
                200,
                serde_json::json!({"kind":"job_cancelled","job_ref":request["job_ref"]}),
            ),
            _ => (404, serde_json::json!({"error":{"code":"not_found"}})),
        }
    })
    .await;
    let value = state
        .dispatch_job_tool(
            Some(&run),
            "chariox.jobs.run",
            &serde_json::json!({"command":["/bin/echo","ran-here"],"needs":{"memory_gib":1},"wait_s":20}),
            &task,
        )
        .await
        .unwrap();
    server.abort();
    value
}

#[tokio::test]
async fn offload_job_follows_the_project_policy_until_the_room_overrides_it() {
    let policy = |state: &KernelRuntimeState, task: &AgentTaskExecution, project, policy| {
        state
            .set_remote_job_policy(&task.room_id, None, project, &task.owner_user_id, policy)
            .unwrap();
    };
    let tail =
        |value: &serde_json::Value| value["result"]["output_tail"].as_str().unwrap().to_owned();
    let by_project = job_refused_by_the_service(409, "capacity_exceeded", |state, task| {
        policy(
            state,
            task,
            true,
            Some(crate::local::OffloadPolicy::AlwaysHeavy),
        );
    })
    .await;
    assert!(
        tail(&by_project).contains("No remote machine free"),
        "{by_project}"
    );
    let overridden = job_refused_by_the_service(409, "capacity_exceeded", |state, task| {
        policy(
            state,
            task,
            true,
            Some(crate::local::OffloadPolicy::AlwaysHeavy),
        );
        policy(state, task, false, Some(crate::local::OffloadPolicy::Off));
    })
    .await;
    assert!(
        !tail(&overridden).contains("No remote machine free"),
        "{overridden}"
    );
    assert!(tail(&overridden).contains("ran-here"), "{overridden}");
}

#[tokio::test]
async fn offload_job_waits_locally_when_the_service_refuses_for_capacity() {
    refused_job_waits_locally(409, "capacity_exceeded", "No remote machine free").await;
}

#[tokio::test]
async fn offload_job_waits_locally_when_the_service_rate_limits_it() {
    refused_job_waits_locally(
        429,
        "rate_limited",
        "rate limited right now; waiting locally",
    )
    .await;
}

#[tokio::test]
async fn offload_job_waits_locally_when_the_service_is_unavailable() {
    refused_job_waits_locally(
        503,
        "dependency_unavailable",
        "service is unavailable right now",
    )
    .await;
}

#[tokio::test]
async fn offload_job_waits_locally_when_the_service_fails_internally() {
    refused_job_waits_locally(500, "internal_error", "service had an internal error").await;
}
