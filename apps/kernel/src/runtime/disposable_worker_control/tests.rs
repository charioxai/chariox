use super::*;
use crate::local::{
    CreateDisposableWorkerRequest, DisposableWorkerRequest, ManagedEnvironmentAutoStopPolicy,
};
use std::io::{Read, Write};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

struct Fixture {
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
    stopped: Arc<AtomicBool>,
    task: Option<std::thread::JoinHandle<()>>,
}
impl Fixture {
    fn new(responses: Vec<serde_json::Value>) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stopped = Arc::new(AtomicBool::new(false));
        let seen = requests.clone();
        let stop = stopped.clone();
        let task = std::thread::spawn(move || {
            let mut responses = responses.into_iter();
            while !stop.load(Ordering::SeqCst) {
                let (mut stream, _) = match listener.accept() {
                    Ok(pair) => pair,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(5));
                        continue;
                    }
                    Err(e) => panic!("{e}"),
                };
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                    .unwrap();
                let mut data = Vec::new();
                loop {
                    let mut chunk = [0; 1024];
                    let n = stream.read(&mut chunk).unwrap();
                    if n == 0 {
                        break;
                    }
                    data.extend_from_slice(&chunk[..n]);
                    if let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&data[..end]).to_lowercase();
                        let size = headers
                            .lines()
                            .find_map(|line| line.strip_prefix("content-length:"))
                            .map(|s| s.trim().parse::<usize>().unwrap())
                            .unwrap_or(0);
                        if data.len() >= end + 4 + size {
                            break;
                        }
                    }
                }
                seen.lock().unwrap().push(String::from_utf8(data).unwrap());
                let body = responses
                    .next()
                    .expect("unexpected extra HTTP call")
                    .to_string();
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
            }
        });
        Self {
            url,
            requests,
            stopped,
            task: Some(task),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
        self.task.take().unwrap().join().unwrap();
    }
}

fn allocation() -> serde_json::Value {
    serde_json::json!({"allocationId":"worker-1","homeKernelId":"home-1","homeRelayRealmId":"realm-1",
        "region":"fsn1","computeClass":"worker","architecture":"x86_64","providerId":"hetzner","providerProfileId":"profile-1",
        "storageMonthlyMinorUnits":100,"managedRepositoryRoot":"/workspace","runtimeReleaseDigest":"sha256:fixture",
        "desiredState":"running","observedState":"ready","desiredRevision":1,"observedRevision":1,
        "runtimeMachineId":"machine-1","runtimeKernelId":"worker-kernel-1","operationId":"operation-1","operationStatus":"succeeded",
        "expiresAt":"2026-09-28T12:00:00Z","createdAt":"2026-09-28T08:00:00Z",
        "autoStopPolicy":{"minimumRuntimeSeconds":12600,"idleDelaySeconds":null},"autoStopDeadlineAt":null})
}
fn config(server: &Fixture) -> DaemonConfig {
    let mut config = DaemonConfig::for_tests();
    config.daemon_id = "home-1".into();
    config.cloud_relay = Some(PersistedCloudRelayProfile {
        kernel_id: None,
        kernel_credential: None,
        kernel_public_key_thumbprint: None,
        account_id: "account-1".into(),
        user_id: "owner-1".into(),
        realm_id: "realm-1".into(),
        api_url: server.url.clone(),
        cloud_session_token: Some("fixture-session".into()),
        ..Default::default()
    });
    config
}
fn selection() -> DisposableWorkerRequest {
    DisposableWorkerRequest {
        allocation_id: "worker-1".into(),
        home_kernel_id: "home-1".into(),
        home_relay_realm_id: "realm-1".into(),
    }
}
async fn execute(
    config: DaemonConfig,
    caller: &str,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let profiles = crate::account_profile::ProviderAccountProfileRegistry::open(
        config.account_profile_registry_path(),
    )
    .unwrap();
    execute_disposable_worker_control_request(config, profiles, caller, request).await
}

#[tokio::test]
async fn unauthorized_or_misrouted_controls_never_reach_cloud() {
    let server = Fixture::new(vec![]);
    let cfg = config(&server);
    let request = LocalDaemonRequest::ReleaseDisposableWorker(selection());
    assert!(execute(cfg.clone(), "other-user", request.clone())
        .await
        .is_err());
    let mut absent = cfg.clone();
    absent.cloud_relay.as_mut().unwrap().cloud_session_token = None;
    assert!(execute(absent, "owner-1", request).await.is_err());
    for (kernel, realm) in [("other-home", "realm-1"), ("home-1", "other-realm")] {
        let mut wrong = selection();
        wrong.home_kernel_id = kernel.into();
        wrong.home_relay_realm_id = realm.into();
        assert!(execute(
            cfg.clone(),
            "owner-1",
            LocalDaemonRequest::KeepDisposableWorkerRunning(wrong)
        )
        .await
        .is_err());
    }
    assert!(server.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn foreign_or_unbound_cloud_allocation_prevents_mutation() {
    for field in ["allocationId", "homeKernelId", "homeRelayRealmId"] {
        let mut wrong = allocation();
        wrong[field] = "other".into();
        let server = Fixture::new(vec![wrong]);
        assert!(execute(
            config(&server),
            "owner-1",
            LocalDaemonRequest::ReleaseDisposableWorker(selection())
        )
        .await
        .is_err());
        assert_eq!(server.requests.lock().unwrap().len(), 1);
    }
    let mut unbound = allocation();
    unbound["homeKernelId"] = serde_json::Value::Null;
    let server = Fixture::new(vec![unbound]);
    assert!(execute(
        config(&server),
        "owner-1",
        LocalDaemonRequest::KeepDisposableWorkerRunning(selection())
    )
    .await
    .is_err());
    assert_eq!(server.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn release_and_keep_running_bind_preflight_and_response_and_use_session_authority() {
    for (request, method) in [
        (
            LocalDaemonRequest::ReleaseDisposableWorker(selection()),
            "DELETE /disposable-workers/worker-1?accountId=account-1",
        ),
        (
            LocalDaemonRequest::KeepDisposableWorkerRunning(selection()),
            "POST /disposable-workers/worker-1/keep-running?accountId=account-1",
        ),
    ] {
        let server = Fixture::new(vec![allocation(), allocation()]);
        assert!(matches!(
            execute(config(&server), "owner-1", request).await.unwrap(),
            LocalDaemonResponse::DisposableWorker { .. }
        ));
        let requests = server.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].starts_with("GET /disposable-workers/worker-1?accountId=account-1 "));
        assert!(requests[1].starts_with(method));
        assert!(requests.iter().all(|r| r
            .to_lowercase()
            .contains("authorization: bearer fixture-session")));
    }
    let mut wrong = allocation();
    wrong["homeKernelId"] = "other".into();
    let server = Fixture::new(vec![allocation(), wrong]);
    assert!(execute(
        config(&server),
        "owner-1",
        LocalDaemonRequest::ReleaseDisposableWorker(selection())
    )
    .await
    .is_err());
}

#[tokio::test]
async fn create_preserves_cloud_idempotency_and_owner_projection_without_retry() {
    let server = Fixture::new(vec![allocation(), allocation()]);
    let cfg = config(&server);
    let request = LocalDaemonRequest::CreateDisposableWorker(CreateDisposableWorkerRequest {
        client_request_id: "stable-request-1".into(),
        home_kernel_id: "home-1".into(),
        home_relay_realm_id: "realm-1".into(),
        region: "fsn1".into(),
        compute_class: "worker".into(),
        architecture: "x86_64".into(),
        maximum_lifetime_seconds: 14400,
        auto_stop_policy: ManagedEnvironmentAutoStopPolicy {
            minimum_runtime_seconds: 12600,
            idle_delay_seconds: None,
        },
        context_plan: None,
    });
    execute(cfg.clone(), "owner-1", request.clone())
        .await
        .unwrap();
    execute(cfg, "owner-1", request).await.unwrap();
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    let bodies: Vec<serde_json::Value> = requests
        .iter()
        .map(|r| serde_json::from_str(r.split("\r\n\r\n").nth(1).unwrap()).unwrap())
        .collect();
    assert_eq!(bodies[0], bodies[1]);
    assert_eq!(bodies[0]["clientRequestId"], "stable-request-1");
    assert_eq!(bodies[0]["accountId"], "account-1");
    assert!(bodies[0].get("actorUserId").is_none());
}

#[test]
fn request_shapes_reject_caller_supplied_account_authority() {
    let mut input = serde_json::to_value(selection()).unwrap();
    input["accountId"] = "other-account".into();
    assert!(serde_json::from_value::<LocalDaemonRequest>(
        serde_json::json!({"GetDisposableWorker":input})
    )
    .is_err());
}

#[tokio::test]
async fn context_transfer_binds_allocation_worker_and_existing_source_authority() {
    let unused = Fixture::new(vec![]);
    let mut cfg = config(&unused);
    cfg.cloud_relay.as_mut().unwrap().machine_id = Some("source-machine-test".into());
    let source_thumbprint =
        crate::runtime::terminal_pairings::public_key_thumbprint(&cfg.relay_public_key);
    let plan = crate::managed_bootstrap::ManagedKernelContextPlan::source_project_for_tests(
        "context-1",
        "realm-1",
        &cfg.daemon_id,
        &source_thumbprint,
        "project-1",
    );
    let private = crate::transport::relay_crypto::generate_private_key_base64();
    let public =
        crate::transport::relay_crypto::public_key_from_private_key_base64(&private).unwrap();
    let thumbprint = crate::runtime::terminal_pairings::public_key_thumbprint(&public);
    let ticket = serde_json::json!({"environmentId":"worker-1","contextPlan":plan,
        "target":{"relayRealmId":"realm-1","machineId":"machine-1","kernelId":"worker-kernel-1","relayPublicKey":public,"keyThumbprint":thumbprint}});
    let server = Fixture::new(vec![allocation(), ticket.clone()]);
    cfg.cloud_relay.as_mut().unwrap().api_url = server.url.clone();
    let response = execute(
        cfg.clone(),
        "owner-1",
        LocalDaemonRequest::PrepareDisposableWorkerContextTransfer(selection()),
    )
    .await
    .unwrap();
    assert!(matches!(
        response,
        LocalDaemonResponse::DisposableWorkerContextTransferPrepared { .. }
    ));
    assert_eq!(server.requests.lock().unwrap().len(), 2);
    for pointer in [
        "/environmentId",
        "/target/kernelId",
        "/target/machineId",
        "/target/relayRealmId",
    ] {
        let mut wrong = ticket.clone();
        *wrong.pointer_mut(pointer).unwrap() = "another".into();
        let server = Fixture::new(vec![allocation(), wrong]);
        cfg.cloud_relay.as_mut().unwrap().api_url = server.url.clone();
        assert!(execute(
            cfg.clone(),
            "owner-1",
            LocalDaemonRequest::PrepareDisposableWorkerContextTransfer(selection())
        )
        .await
        .is_err());
    }
}

#[tokio::test]
async fn managed_keep_running_binds_environment_and_uses_normal_cloud_route() {
    let environment = serde_json::json!({"environmentId":"environment-1","accountId":"account-1","createdByUserId":"owner-1",
        "name":"Managed","region":"fsn1","computeClass":"worker","managedRepositoryRoot":"/workspace",
        "desiredState":"running","observedState":"ready","desiredRevision":1,"observedRevision":1,
        "runtimeMachineId":null,"runtimeKernelId":null,"runtimeReleaseDigest":null,
        "contextPlan":{"schemaVersion":1,"contextId":"context-1","planDigest":"sha256:plan","source":null,"kernelContext":"empty",
            "developmentSetup":{"kind":"empty"},"providerAccounts":{"kind":"none"},"gitCredentials":{"kind":"none"}},
        "contextManifestDigest":null,"autoStopPolicy":{"minimumRuntimeSeconds":0,"idleDelaySeconds":null},
        "runningAgentCount":0,"lastActivityReportedAt":null,"lastActivityChangedAt":null,"autoStopWarningAt":null,"autoStopDeadlineAt":null,
        "lastErrorCode":null,"lastErrorMessage":null,"createdAt":"2026-09-28T08:00:00Z","updatedAt":"2026-09-28T08:00:00Z"});
    let request = LocalDaemonRequest::KeepManagedEnvironmentRunning(
        crate::local::GetManagedEnvironmentRequest {
            environment_id: "environment-1".into(),
        },
    );
    let server = Fixture::new(vec![serde_json::json!({"environment":environment})]);
    assert!(matches!(
        execute(config(&server), "owner-1", request.clone())
            .await
            .unwrap(),
        LocalDaemonResponse::ManagedEnvironmentKeptRunning { .. }
    ));
    {
        let seen = server.requests.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert!(
            seen[0].starts_with("POST /managed-environments/environment-1/auto-stop/keep-running ")
        );
        assert!(seen[0].ends_with("{\"accountId\":\"account-1\"}"));
    }
    let mut wrong = environment;
    wrong["environmentId"] = "another".into();
    let server = Fixture::new(vec![serde_json::json!({"environment":wrong})]);
    assert!(execute(config(&server), "owner-1", request).await.is_err());
}
