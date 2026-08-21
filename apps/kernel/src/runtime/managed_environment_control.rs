use crate::config::{DaemonConfig, PersistedCloudRelayProfile};
use crate::error::DaemonError;
use crate::local::{LocalDaemonRequest, LocalDaemonResponse, ManagedEnvironmentCatalog};
use crate::runtime::cloud_api_client::{
    cloud_url_component, get_cloud_json_authenticated, post_cloud_json_authenticated,
};

mod cloud_contract;
use cloud_contract::{
    EnvironmentDetailsResponse, EnvironmentResult, EnvironmentsResponse, OptionsResponse,
};

pub(crate) async fn execute_managed_environment_control_request(
    config: DaemonConfig,
    caller_user_id: &str,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let cloud = authorized_cloud_profile(&config, caller_user_id)?;
    let token = cloud
        .cloud_session_token
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| control_error("Cloud session is unavailable"))?;
    let account_id = cloud.account_id.trim();
    if account_id.is_empty() {
        return Err(control_error("Cloud account is unavailable"));
    }
    let account_query = format!("accountId={}", cloud_url_component(account_id));

    match request {
        LocalDaemonRequest::ListManagedEnvironmentCatalog(_) => {
            let options_path = format!("/managed-environments/options?{account_query}");
            let environments_path = format!("/managed-environments?{account_query}");
            let (options, environments) = tokio::try_join!(
                get_cloud_json_authenticated::<OptionsResponse>(
                    cloud.api_url.clone(),
                    options_path,
                    token.to_string(),
                ),
                get_cloud_json_authenticated::<EnvironmentsResponse>(
                    cloud.api_url.clone(),
                    environments_path,
                    token.to_string(),
                ),
            )?;
            Ok(LocalDaemonResponse::ManagedEnvironmentCatalog {
                catalog: ManagedEnvironmentCatalog {
                    compute_classes: options
                        .compute_classes
                        .into_iter()
                        .map(Into::into)
                        .collect(),
                    context_sources: options
                        .context_sources
                        .into_iter()
                        .map(Into::into)
                        .collect(),
                    environments: environments
                        .environments
                        .into_iter()
                        .map(Into::into)
                        .collect(),
                },
            })
        }
        LocalDaemonRequest::GetManagedEnvironment(request) => {
            let path = format!(
                "/managed-environments/{}?{account_query}",
                cloud_url_component(&request.environment_id),
            );
            let response = get_cloud_json_authenticated::<EnvironmentDetailsResponse>(
                cloud.api_url.clone(),
                path,
                token.to_string(),
            )
            .await?;
            Ok(LocalDaemonResponse::ManagedEnvironment {
                environment: response.environment.into(),
            })
        }
        LocalDaemonRequest::CreateManagedEnvironment(request) => {
            let mut body =
                serde_json::to_value(request).map_err(|error| DaemonError::LocalTransport {
                    operation: "encode managed environment create request",
                    message: error.to_string(),
                })?;
            body.as_object_mut()
                .ok_or_else(|| control_error("managed environment create request is invalid"))?
                .insert(
                    "accountId".to_string(),
                    serde_json::Value::String(account_id.to_string()),
                );
            let result = post_cloud_json_authenticated::<EnvironmentResult>(
                cloud.api_url.clone(),
                "/managed-environments".to_string(),
                token.to_string(),
                body,
            )
            .await?;
            Ok(LocalDaemonResponse::ManagedEnvironmentCreated {
                result: result.into(),
            })
        }
        LocalDaemonRequest::RequestManagedEnvironmentLifecycle(request) => {
            let path = format!(
                "/managed-environments/{}/lifecycle",
                cloud_url_component(&request.environment_id),
            );
            let body = serde_json::json!({
                "accountId": account_id,
                "action": request.action,
                "idempotencyKey": request.idempotency_key,
            });
            let result = post_cloud_json_authenticated::<EnvironmentResult>(
                cloud.api_url.clone(),
                path,
                token.to_string(),
                body,
            )
            .await?;
            Ok(LocalDaemonResponse::ManagedEnvironmentLifecycleRequested {
                result: result.into(),
            })
        }
        _ => Err(DaemonError::LocalTransport {
            operation: "managed environment control",
            message: "unsupported request".to_string(),
        }),
    }
}

fn authorized_cloud_profile<'a>(
    config: &'a DaemonConfig,
    caller_user_id: &str,
) -> Result<&'a PersistedCloudRelayProfile, DaemonError> {
    let cloud = config
        .cloud_relay
        .as_ref()
        .ok_or_else(|| control_error("kernel is not connected to Chariox Cloud"))?;
    if caller_user_id != cloud.user_id {
        return Err(control_error(
            "managed environment control belongs to another Cloud user",
        ));
    }
    Ok(cloud)
}

fn control_error(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "managed environment control",
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::{
        CreateManagedEnvironmentRequest, GetManagedEnvironmentRequest,
        ListManagedEnvironmentCatalogRequest, ManagedEnvironmentAutoStopPolicy,
        ManagedEnvironmentContextPlanInput, ManagedEnvironmentDevelopmentSetup,
        ManagedEnvironmentGitCredentials, ManagedEnvironmentKernelContextSelection,
        ManagedEnvironmentLifecycleAction, ManagedEnvironmentProviderAccounts,
        RequestManagedEnvironmentLifecycleRequest,
    };
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    #[test]
    fn managed_environment_control_rejects_callers_without_the_cloud_user_identity() {
        let mut config = DaemonConfig::for_tests();
        assert!(authorized_cloud_profile(&config, crate::session::DEFAULT_LOCAL_USER_ID).is_err());
        config.cloud_relay = Some(PersistedCloudRelayProfile {
            user_id: "cloud-user-1".to_string(),
            ..PersistedCloudRelayProfile::default()
        });

        authorized_cloud_profile(&config, "cloud-user-1").expect("Cloud owner");
        assert!(authorized_cloud_profile(&config, crate::session::DEFAULT_LOCAL_USER_ID).is_err());
        assert!(authorized_cloud_profile(&config, "cloud-user-2").is_err());
    }

    #[tokio::test]
    async fn managed_environment_control_uses_authenticated_cloud_profile_for_all_operations() {
        let server = ManagedEnvironmentCloudFixture::start();
        let mut config = DaemonConfig::for_tests();
        config.cloud_relay = Some(PersistedCloudRelayProfile {
            api_url: server.url(),
            account_id: "account / one".to_string(),
            user_id: "cloud-user-1".to_string(),
            cloud_session_token: Some("session-secret".to_string()),
            ..PersistedCloudRelayProfile::default()
        });

        let catalog = execute_managed_environment_control_request(
            config.clone(),
            "cloud-user-1",
            LocalDaemonRequest::ListManagedEnvironmentCatalog(ListManagedEnvironmentCatalogRequest),
        )
        .await
        .expect("catalog request");
        let LocalDaemonResponse::ManagedEnvironmentCatalog { catalog } = catalog else {
            panic!("unexpected catalog response");
        };
        assert_eq!(catalog.compute_classes[0].compute_class, "agent-small");
        assert_eq!(catalog.context_sources[0].source_target_id, "source-1");
        assert_eq!(catalog.environments[0].environment_id, "environment-1");

        let create = execute_managed_environment_control_request(
            config.clone(),
            "cloud-user-1",
            LocalDaemonRequest::CreateManagedEnvironment(CreateManagedEnvironmentRequest {
                client_request_id: "create-1".to_string(),
                name: "Managed agent".to_string(),
                region: "hel1".to_string(),
                compute_class: "agent-small".to_string(),
                auto_stop_policy: ManagedEnvironmentAutoStopPolicy {
                    minimum_runtime_seconds: 0,
                    idle_delay_seconds: Some(900),
                },
                context_plan: ManagedEnvironmentContextPlanInput {
                    source_target_id: None,
                    kernel_context: ManagedEnvironmentKernelContextSelection::Empty,
                    development_setup: ManagedEnvironmentDevelopmentSetup::Empty,
                    provider_accounts: ManagedEnvironmentProviderAccounts::None,
                    git_credentials: ManagedEnvironmentGitCredentials::None,
                },
            }),
        )
        .await
        .expect("create request");
        assert!(matches!(
            create,
            LocalDaemonResponse::ManagedEnvironmentCreated { .. }
        ));

        let get = execute_managed_environment_control_request(
            config.clone(),
            "cloud-user-1",
            LocalDaemonRequest::GetManagedEnvironment(GetManagedEnvironmentRequest {
                environment_id: "environment / one".to_string(),
            }),
        )
        .await
        .expect("get request");
        assert!(matches!(
            get,
            LocalDaemonResponse::ManagedEnvironment { .. }
        ));

        let lifecycle = execute_managed_environment_control_request(
            config,
            "cloud-user-1",
            LocalDaemonRequest::RequestManagedEnvironmentLifecycle(
                RequestManagedEnvironmentLifecycleRequest {
                    environment_id: "environment / one".to_string(),
                    action: ManagedEnvironmentLifecycleAction::Start,
                    idempotency_key: "start-1".to_string(),
                },
            ),
        )
        .await
        .expect("lifecycle request");
        assert!(matches!(
            lifecycle,
            LocalDaemonResponse::ManagedEnvironmentLifecycleRequested { .. }
        ));

        let requests = server.requests();
        assert_eq!(requests.len(), 5);
        assert!(requests.iter().all(|request| request
            .to_ascii_lowercase()
            .contains("authorization: bearer session-secret")));
        assert!(requests.iter().any(|request| request.starts_with(
            "GET /managed-environments/options?accountId=account%20%2F%20one HTTP/1.1"
        )));
        assert!(requests.iter().any(|request| request
            .starts_with("GET /managed-environments?accountId=account%20%2F%20one HTTP/1.1")));
        assert!(requests.iter().any(|request| request.starts_with(
            "GET /managed-environments/environment%20%2F%20one?accountId=account%20%2F%20one HTTP/1.1"
        )));
        let create_request = requests
            .iter()
            .find(|request| request.starts_with("POST /managed-environments HTTP/1.1"))
            .expect("create HTTP request");
        assert!(create_request.contains(r#""accountId":"account / one""#));
        assert!(create_request.contains(r#""clientRequestId":"create-1""#));
        let lifecycle_request = requests
            .iter()
            .find(|request| request.contains("/lifecycle HTTP/1.1"))
            .expect("lifecycle HTTP request");
        assert!(lifecycle_request.contains(r#""action":"start""#));
        assert!(lifecycle_request.contains(r#""idempotencyKey":"start-1""#));
        assert!(!requests.join("\n").contains("session-secret\""));
    }

    struct ManagedEnvironmentCloudFixture {
        address: std::net::SocketAddr,
        stop: Arc<AtomicBool>,
        requests: Arc<Mutex<Vec<String>>>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl ManagedEnvironmentCloudFixture {
        fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind Cloud fixture");
            listener.set_nonblocking(true).expect("nonblocking fixture");
            let address = listener.local_addr().expect("fixture address");
            let stop = Arc::new(AtomicBool::new(false));
            let requests = Arc::new(Mutex::new(Vec::new()));
            let thread_stop = Arc::clone(&stop);
            let thread_requests = Arc::clone(&requests);
            let thread = std::thread::spawn(move || {
                while !thread_stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            let request = read_http_request(&mut stream);
                            if request.is_empty() {
                                continue;
                            }
                            let response = fixture_response(&request);
                            thread_requests.lock().expect("requests lock").push(request);
                            write_http_response(&mut stream, &response);
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(2));
                        }
                        Err(error) => panic!("Cloud fixture failed: {error}"),
                    }
                }
            });
            Self {
                address,
                stop,
                requests,
                thread: Some(thread),
            }
        }

        fn url(&self) -> String {
            format!("http://{}", self.address)
        }

        fn requests(&self) -> Vec<String> {
            self.requests.lock().expect("requests lock").clone()
        }
    }

    impl Drop for ManagedEnvironmentCloudFixture {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            let _ = TcpStream::connect(self.address);
            if let Some(thread) = self.thread.take() {
                thread.join().expect("Cloud fixture should stop");
            }
        }
    }

    fn read_http_request(stream: &mut TcpStream) -> String {
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("fixture timeout");
        let mut request = Vec::new();
        let mut buffer = [0_u8; 4096];
        loop {
            let read = stream.read(&mut buffer).expect("read fixture request");
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
            if http_request_complete(&request) {
                break;
            }
        }
        String::from_utf8(request).expect("fixture request UTF-8")
    }

    fn http_request_complete(request: &[u8]) -> bool {
        let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n") else {
            return false;
        };
        let headers = String::from_utf8_lossy(&request[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .unwrap_or(0);
        request.len() >= header_end + 4 + content_length
    }

    fn fixture_response(request: &str) -> serde_json::Value {
        if request.starts_with("GET /managed-environments/options?") {
            return serde_json::json!({
                "computeClasses": [{
                    "computeClass": "agent-small",
                    "regions": ["hel1"],
                    "futureComputeField": true
                }],
                "contextSources": [{
                    "sourceTargetId": "source-1",
                    "machineId": "source-machine",
                    "kernelId": "source-kernel",
                    "label": "This machine",
                    "futureSourceField": true
                }],
                "futureOptionsField": true
            });
        }
        if request.starts_with("GET /managed-environments?") {
            return serde_json::json!({ "environments": [environment_json()] });
        }
        if request.starts_with("GET /managed-environments/") {
            return serde_json::json!({
                "environment": environment_json(),
                "operations": [],
                "futureDetailsField": true
            });
        }
        if request.starts_with("POST /managed-environments") {
            return serde_json::json!({
                "environment": environment_json(),
                "operation": operation_json(),
                "futureResultField": true
            });
        }
        panic!("unexpected Cloud fixture request: {request}");
    }

    fn environment_json() -> serde_json::Value {
        serde_json::json!({
            "environmentId": "environment-1",
            "accountId": "account / one",
            "createdByUserId": "cloud-user-1",
            "name": "Managed agent",
            "region": "hel1",
            "computeClass": "agent-small",
            "desiredState": "running",
            "observedState": "ready",
            "desiredRevision": 1,
            "observedRevision": 1,
            "runtimeMachineId": "managed-machine-1",
            "runtimeReleaseDigest": "sha256:release",
            "contextPlan": {
                "schemaVersion": 1,
                "contextId": "context-1",
                "planDigest": "sha256:plan",
                "source": null,
                "kernelContext": "empty",
                "developmentSetup": { "kind": "empty", "futureDevelopmentField": true },
                "providerAccounts": { "kind": "none" },
                "gitCredentials": { "kind": "none" },
                "futurePlanField": true
            },
            "contextManifestDigest": "sha256:manifest",
            "autoStopPolicy": { "minimumRuntimeSeconds": 0, "idleDelaySeconds": 900 },
            "lastErrorCode": null,
            "lastErrorMessage": null,
            "createdAt": "2026-08-21T00:00:00.000Z",
            "updatedAt": "2026-08-21T00:00:00.000Z",
            "futureEnvironmentField": true
        })
    }

    fn operation_json() -> serde_json::Value {
        serde_json::json!({
            "operationId": "operation-1",
            "environmentId": "environment-1",
            "requestedByUserId": "cloud-user-1",
            "kind": "create",
            "idempotencyKey": "create-1",
            "requestDigest": "sha256:request",
            "desiredRevision": 1,
            "status": "pending",
            "attempt": 0,
            "retryable": false,
            "failureCode": null,
            "failureMessage": null,
            "completedAt": null,
            "createdAt": "2026-08-21T00:00:00.000Z",
            "updatedAt": "2026-08-21T00:00:00.000Z",
            "futureOperationField": true
        })
    }

    fn write_http_response(stream: &mut TcpStream, body: &serde_json::Value) {
        let body = body.to_string();
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            body.len(),
            body,
        );
        stream
            .write_all(response.as_bytes())
            .expect("write fixture response");
    }
}
