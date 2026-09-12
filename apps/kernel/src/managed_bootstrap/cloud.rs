use std::io::Read;
use std::time::Duration;

use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::error::DaemonError;

use super::context_plan::ManagedKernelContextPlan;

const MAX_RESPONSE_BYTES: u64 = 96 * 1024;
const DISPOSABLE_WORKER_EXCHANGE_PATH: &str = "/v1/disposable-workers/bootstrap/exchange";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ExchangeRequest {
    pub(super) token: String,
    pub(super) environment_id: String,
    pub(super) machine_id: String,
    pub(super) kernel_id: String,
    pub(super) relay_public_key: String,
    pub(super) runtime_release_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ExchangeResponse {
    pub(super) environment_id: String,
    pub(super) kernel_id: String,
    pub(super) runtime_release_digest: String,
    pub(super) context_plan: ManagedKernelContextPlan,
    pub(super) cloud_relay: ManagedCloudRelayProfile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DisposableWorkerExchangeRequest {
    pub(super) token: String,
    pub(super) allocation_id: String,
    pub(super) worker_machine_id: String,
    pub(super) worker_kernel_id: String,
    pub(super) image_digest: String,
    pub(super) runtime_release_digest: String,
    pub(super) manager_operation_id: String,
    pub(super) manager_operation_fence: u64,
    pub(super) manager_request_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct DisposableWorkerEnrollmentReceipt {
    pub(super) grant_id: String,
    pub(super) allocation_id: String,
    pub(super) worker_machine_id: String,
    pub(super) worker_kernel_id: String,
    pub(super) image_digest: String,
    pub(super) runtime_release_digest: String,
    pub(super) exchanged_at: String,
}

// This is an internal supervisor result, not a Cloud wire contract. A client may
// construct it only after Cloud authenticated the registered home-kernel sender
// and returned the exact enrollment receipt plus the worker's relay profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DisposableWorkerBootstrapResult {
    pub(super) enrollment_receipt: DisposableWorkerEnrollmentReceipt,
    pub(super) cloud_relay: ManagedCloudRelayProfile,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DisposableWorkerExchangeOutcome {
    Accepted(DisposableWorkerEnrollmentReceipt),
    Pending,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DisposableWorkerRecoveryOutcome {
    Ready(DisposableWorkerBootstrapResult),
    Pending,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ManagedCloudRelayProfile {
    pub(super) api_url: String,
    pub(super) email: String,
    pub(super) account_id: String,
    pub(super) user_id: String,
    pub(super) account_slug: String,
    pub(super) realm_id: String,
    pub(super) relay_url: String,
    pub(super) issuer_id: String,
    pub(super) machine_id: String,
    pub(super) machine_alias: String,
    pub(super) machine_credential: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ConfirmRequest {
    pub(super) token: String,
    pub(super) environment_id: String,
    pub(super) machine_id: String,
    pub(super) machine_credential: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ConfirmResponse {
    pub(super) confirmed: bool,
    pub(super) observed_state: String,
}

pub(super) trait BootstrapCloudClient {
    fn exchange(
        &self,
        api_url: &str,
        request: &ExchangeRequest,
    ) -> Result<ExchangeResponse, DaemonError>;
    fn exchange_disposable_worker(
        &self,
        api_url: &str,
        request: &DisposableWorkerExchangeRequest,
    ) -> Result<DisposableWorkerExchangeOutcome, DaemonError> {
        let _ = (api_url, request);
        Err(cloud_error(
            "disposable worker bootstrap requires verified home-kernel sender proof",
        ))
    }
    fn recover_disposable_worker_exchange(
        &self,
        api_url: &str,
        binding_digest: &str,
        binding: &super::state::DisposableWorkerBinding,
    ) -> Result<DisposableWorkerRecoveryOutcome, DaemonError> {
        let _ = (api_url, binding_digest, binding);
        Err(cloud_error(
            "disposable worker bootstrap recovery requires verified home-kernel sender proof",
        ))
    }
    fn confirm(
        &self,
        api_url: &str,
        request: &ConfirmRequest,
    ) -> Result<ConfirmResponse, DaemonError>;
}

pub(super) struct HttpBootstrapCloudClient {
    agent: ureq::Agent,
}

impl Default for HttpBootstrapCloudClient {
    fn default() -> Self {
        Self {
            agent: ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(20))
                .build(),
        }
    }
}

impl BootstrapCloudClient for HttpBootstrapCloudClient {
    fn exchange(
        &self,
        api_url: &str,
        request: &ExchangeRequest,
    ) -> Result<ExchangeResponse, DaemonError> {
        self.post_managed(api_url, "/v1/managed-kernels/bootstrap/exchange", request)
    }

    fn exchange_disposable_worker(
        &self,
        api_url: &str,
        request: &DisposableWorkerExchangeRequest,
    ) -> Result<DisposableWorkerExchangeOutcome, DaemonError> {
        let _ = (api_url, request, DISPOSABLE_WORKER_EXCHANGE_PATH);
        Err(cloud_error(
            "disposable worker exchange is unavailable until Cloud supplies verified \
             home-kernel sender proof and idempotent result recovery",
        ))
    }

    fn recover_disposable_worker_exchange(
        &self,
        api_url: &str,
        binding_digest: &str,
        binding: &super::state::DisposableWorkerBinding,
    ) -> Result<DisposableWorkerRecoveryOutcome, DaemonError> {
        let _ = (api_url, binding_digest, binding);
        Err(cloud_error(
            "disposable worker exchange recovery is unavailable until Cloud supplies an \
             authenticated result endpoint",
        ))
    }

    fn confirm(
        &self,
        api_url: &str,
        request: &ConfirmRequest,
    ) -> Result<ConfirmResponse, DaemonError> {
        self.post_managed(api_url, "/v1/managed-kernels/bootstrap/confirm", request)
    }
}

impl HttpBootstrapCloudClient {
    fn post_managed<T: DeserializeOwned>(
        &self,
        api_url: &str,
        path: &str,
        request: &impl Serialize,
    ) -> Result<T, DaemonError> {
        self.post(api_url, path, request)
            .map_err(|error| match error {
                PostError::Rejected => cloud_error("Cloud bootstrap request was rejected"),
                PostError::Failure(error) => error,
            })
    }

    fn post<T: DeserializeOwned>(
        &self,
        api_url: &str,
        path: &str,
        request: &impl Serialize,
    ) -> Result<T, PostError> {
        let body = serde_json::to_string(request)
            .map_err(|error| PostError::Failure(cloud_error(error.to_string())))?;
        let response = self
            .agent
            .post(&format!("{api_url}{path}"))
            .set("content-type", "application/json")
            .send_string(&body)
            .map_err(map_http_error)?;
        let mut bytes = Vec::new();
        response
            .into_reader()
            .take(MAX_RESPONSE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| PostError::Failure(cloud_error(error.to_string())))?;
        if bytes.len() as u64 > MAX_RESPONSE_BYTES {
            return Err(PostError::Failure(cloud_error(
                "Cloud bootstrap response is too large",
            )));
        }
        serde_json::from_slice(&bytes)
            .map_err(|error| PostError::Failure(cloud_error(error.to_string())))
    }
}

enum PostError {
    Rejected,
    Failure(DaemonError),
}

fn map_http_error(error: ureq::Error) -> PostError {
    match error {
        ureq::Error::Status(status, _) if matches!(status, 400 | 401 | 403 | 409 | 410 | 422) => {
            PostError::Rejected
        }
        ureq::Error::Status(status, _) => PostError::Failure(cloud_error(format!(
            "Cloud bootstrap request failed with HTTP {status}"
        ))),
        ureq::Error::Transport(error) => PostError::Failure(cloud_error(error.to_string())),
    }
}

fn cloud_error(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "managed kernel Cloud bootstrap",
        message: message.into(),
    }
}
