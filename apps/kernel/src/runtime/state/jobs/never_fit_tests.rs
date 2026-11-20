//! A job that can never run on this machine is refused once no other machine takes it; one that
//! fits keeps waiting, and one another machine takes is offloaded.
use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

type Seen = Arc<std::sync::Mutex<Vec<String>>>;

/// A machine service that answers a capabilities ask with `capabilities` and every other call
/// with a job machine in `machine_state`; it records the actions it was asked.
async fn machine_service(
    state: &KernelRuntimeState,
    capabilities: serde_json::Value,
    machine_state: &'static str,
) -> (tokio::task::JoinHandle<()>, Seen) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut config = state.owned.config_projection.snapshot();
    config.relay_url = Some("wss://relay.test".into());
    config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
        kernel_id: Some(config.daemon_id.clone()),
        kernel_credential: Some("synthetic-test-credential".into()),
        kernel_public_key_thumbprint: None,
        api_url: format!("http://{}", listener.local_addr().unwrap()),
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
        token_expires_at_ms: None,
    });
    state.owned.config_projection.update(config);
    let seen = Seen::default();
    let asked = seen.clone();
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
            let length: usize = std::str::from_utf8(&bytes[..header_end])
                .unwrap()
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
            let response = if action == "capabilities" {
                capabilities.clone()
            } else {
                super::cleanup_tests::machine(machine_state)
            }
            .to_string();
            asked.lock().unwrap().push(action);
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).as_bytes()).await.unwrap();
        }
    });
    (server, seen)
}

fn assert_asked_without_acquiring(seen: &Seen) {
    let seen = seen.lock().unwrap();
    assert!(
        seen.iter().any(|action| action == "capabilities"),
        "refused before asking: {seen:?}"
    );
    assert!(
        !seen.iter().any(|action| action == "acquire_job"),
        "{seen:?}"
    );
}

fn no_machines() -> serde_json::Value {
    serde_json::json!({"kind":"capabilities","machine_allocation":null})
}

fn job(needs: serde_json::Value) -> serde_json::Value {
    serde_json::json!({"command":["/bin/true"],"needs":needs,"wait_s":10})
}

fn assert_refused(value: &serde_json::Value, needs: &str, answer: &str) {
    assert_eq!(
        value["result"]["outcome"], "capacity_unavailable",
        "{value}"
    );
    let tail = value["result"]["output_tail"].as_str().unwrap();
    for part in [
        needs,
        "this machine has",
        "can never run here",
        answer,
        "add a machine with",
    ] {
        assert!(tail.contains(part), "{part:?} missing: {tail}");
    }
}

async fn run(
    (state, run, task, _worktree): &super::tests::Fixture,
    request: serde_json::Value,
) -> serde_json::Value {
    state
        .dispatch_job_tool(Some(run), "chariox.jobs.run", &request, task)
        .await
        .unwrap()
}

#[tokio::test]
async fn offload_job_larger_than_the_machine_with_remote_jobs_off_is_refused_with_its_reason() {
    let fixture = super::tests::fixture().await;
    let value = run(&fixture, job(serde_json::json!({"memory_gib": u32::MAX}))).await;
    assert_refused(
        &value,
        "4294967295 GB of memory",
        "remote jobs are switched off",
    );
}

#[tokio::test]
async fn offload_job_with_more_cpus_than_the_machine_is_refused_with_its_reason() {
    let fixture = super::tests::fixture().await;
    let value = run(&fixture, job(serde_json::json!({"cpus": u32::MAX}))).await;
    assert_refused(&value, "4294967295 CPUs", "remote jobs are switched off");
}

#[tokio::test]
async fn offload_job_larger_than_the_machine_is_refused_after_the_service_has_no_machine_for_it() {
    let fixture = super::tests::remote_fixture().await;
    let (server, seen) = machine_service(&fixture.0, no_machines(), "deleted").await;
    let value = run(&fixture, job(serde_json::json!({"memory_gib": u32::MAX}))).await;
    server.abort();
    assert_refused(
        &value,
        "4294967295 GB of memory",
        "no other machine can take it",
    );
    assert_asked_without_acquiring(&seen);
}

#[tokio::test]
async fn offload_job_larger_than_the_machine_asks_for_a_machine_even_with_offloading_off() {
    let fixture = super::tests::remote_fixture().await;
    let task = &fixture.2;
    fixture
        .0
        .set_remote_job_policy(
            &task.room_id,
            Some(&task.agent_id),
            false,
            &task.owner_user_id,
            Some(crate::local::OffloadPolicy::Off),
        )
        .unwrap();
    let (server, seen) = machine_service(&fixture.0, no_machines(), "deleted").await;
    let value = run(&fixture, job(serde_json::json!({"memory_gib": u32::MAX}))).await;
    server.abort();
    assert_refused(
        &value,
        "4294967295 GB of memory",
        "no other machine can take it",
    );
    assert_asked_without_acquiring(&seen);
}

#[tokio::test]
async fn offload_job_larger_than_the_machine_is_refused_when_the_service_cannot_be_reached() {
    let fixture = super::tests::remote_fixture().await;
    let (server, _) = machine_service(&fixture.0, no_machines(), "deleted").await;
    server.abort();
    let _ = server.await;
    let value = run(&fixture, job(serde_json::json!({"memory_gib": u32::MAX}))).await;
    assert_refused(
        &value,
        "4294967295 GB of memory",
        "asking another machine failed",
    );
}

#[tokio::test]
async fn offload_job_larger_than_the_machine_goes_to_a_machine_that_can_take_it() {
    let fixture = super::tests::remote_fixture().await;
    let (state, _, task, _) = &fixture;
    let gib = 100_000;
    let capabilities = serde_json::json!({"kind":"capabilities","machine_allocation":{
        "revision":"v1","actions":["acquire_job"],"offer":null,"job_default_class_id":"big",
        "job_offers":[{"class_id":"big","vcpu":64,"usable_memory_mib":gib * 1024 * 2,"disk_gib":400,
        "rate_per_minute":{"amount_micros":600,"currency":"USD"}}]}});
    let (server, seen) = machine_service(state, capabilities, "creating").await;
    let value = run(
        &fixture,
        serde_json::json!({
            "command":["/bin/true"],"needs":{"memory_gib": gib},"wait_s":0
        }),
    )
    .await;
    let id = value["job_id"].as_str().unwrap();
    let driver = state.job_wake_driver.as_ref().unwrap();
    // The service offers a machine, so the job waits for it (here for the owner's approval).
    tokio::time::timeout(Duration::from_secs(10), async {
        while driver.read(id).unwrap().state != "allocating" {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the job never went looking for a machine");
    let record = driver.read(id).unwrap();
    assert!(
        record.result.is_none(),
        "refused instead of offloaded: {:?}",
        record.result
    );
    assert_eq!(record.placement, JobPlacement::Remote);
    assert!(seen
        .lock()
        .unwrap()
        .iter()
        .any(|action| action == "capabilities"));
    let owner = crate::runtime::command::KernelCaller {
        connection_class: Some(crate::local::KernelConnectionClass::Terminal),
        user_id: Some(task.owner_user_id.clone()),
        ..crate::runtime::command::KernelCaller::default()
    };
    state.cancel_session_job(&task.room_id, &owner, id).unwrap();
    server.abort();
}

/// The scripted service ends once the kernel has asked it to confirm the deletion.
async fn asked_for_cleanup(server: tokio::task::JoinHandle<()>) {
    tokio::time::timeout(Duration::from_secs(10), server)
        .await
        .expect("the kernel never asked the service to finish the cleanup")
        .unwrap();
}

/// The owner accepts the one card that asks to rent the machine.
async fn approve_the_acquisition((state, _, task, _): &super::tests::Fixture) {
    let card = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let session = state
                .owned
                .session_store
                .get_session(&task.room_id)
                .unwrap();
            if let Some(card) = session.active_interactions().first() {
                break card.id().to_owned();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the owner was never asked to rent the machine");
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
}

/// An oversized job is offered a machine that never becomes ready, and the service cannot yet
/// confirm the machine's deletion: the job is refused while its allocation receipt stays saved.
async fn refused_with_unconfirmed_cleanup(fixture: &super::tests::Fixture) -> String {
    let (state, _, _, _) = fixture;
    let gib = 100_000;
    let capabilities = serde_json::json!({"kind":"capabilities","machine_allocation":{
        "revision":"v1","actions":["acquire_job"],"offer":null,"job_default_class_id":"big",
        "job_offers":[{"class_id":"big","vcpu":64,"usable_memory_mib":gib * 1024 * 2,"disk_gib":400,
        "rate_per_minute":{"amount_micros":600,"currency":"USD"}}]}});
    // Every call after the offer finds the machine failed and the cancellation unconfirmed.
    let (server, _) = machine_service(state, capabilities, "failed").await;
    let (value, ()) = tokio::join!(
        run(fixture, job(serde_json::json!({"memory_gib": gib}))),
        approve_the_acquisition(fixture),
    );
    server.abort();
    assert_eq!(
        value["result"]["outcome"], "capacity_unavailable",
        "{value}"
    );
    assert_eq!(value["deleted"], false, "{value}");
    let id = value["job_id"].as_str().unwrap().to_owned();
    let record = state.job_wake_driver.as_ref().unwrap().read(&id).unwrap();
    assert_eq!(record.allocation_ref.as_deref(), Some("cleanup-test"));
    id
}

#[tokio::test]
async fn offload_job_refused_with_unconfirmed_cleanup_is_deleted_by_a_later_status() {
    let fixture = super::tests::remote_fixture().await;
    let id = refused_with_unconfirmed_cleanup(&fixture).await;
    let (state, run, task, _) = &fixture;
    let server = super::cleanup_tests::service(
        state,
        vec![("get", 200, super::cleanup_tests::machine("deleted"))],
    )
    .await;
    let status = state
        .dispatch_job_tool(
            Some(run),
            "chariox.jobs.status",
            &serde_json::json!({"job_id": id}),
            task,
        )
        .await
        .unwrap();
    asked_for_cleanup(server).await;
    assert_eq!(status["deleted"], true, "{status}");
    assert_eq!(status["result"]["outcome"], "capacity_unavailable");
}

#[tokio::test]
async fn offload_job_refused_with_unconfirmed_cleanup_is_deleted_by_a_later_cancel() {
    let fixture = super::tests::remote_fixture().await;
    let id = refused_with_unconfirmed_cleanup(&fixture).await;
    let (state, run, task, _) = &fixture;
    let server = super::cleanup_tests::service(
        state,
        vec![("get", 200, super::cleanup_tests::machine("deleted"))],
    )
    .await;
    let cancelled = state
        .dispatch_job_tool(
            Some(run),
            "chariox.jobs.cancel",
            &serde_json::json!({"job_id": id}),
            task,
        )
        .await
        .unwrap();
    asked_for_cleanup(server).await;
    assert_eq!(cancelled["deleted"], true, "{cancelled}");
    assert_eq!(cancelled["result"]["outcome"], "capacity_unavailable");
}
