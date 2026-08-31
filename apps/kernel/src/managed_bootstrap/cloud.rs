use std::io::Read;
use std::time::Duration;

use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::error::DaemonError;

use super::context_plan::ManagedKernelContextPlan;

const MAX_RESPONSE_BYTES: u64 = 96 * 1024;

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

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
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
        self.post(api_url, "/v1/managed-kernels/bootstrap/exchange", request)
    }

    fn confirm(
        &self,
        api_url: &str,
        request: &ConfirmRequest,
    ) -> Result<ConfirmResponse, DaemonError> {
        self.post(api_url, "/v1/managed-kernels/bootstrap/confirm", request)
    }
}

impl HttpBootstrapCloudClient {
    fn post<T: DeserializeOwned>(
        &self,
        api_url: &str,
        path: &str,
        request: &impl Serialize,
    ) -> Result<T, DaemonError> {
        let body =
            serde_json::to_string(request).map_err(|error| cloud_error(error.to_string()))?;
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
            .map_err(|error| cloud_error(error.to_string()))?;
        if bytes.len() as u64 > MAX_RESPONSE_BYTES {
            return Err(cloud_error("Cloud bootstrap response is too large"));
        }
        serde_json::from_slice(&bytes).map_err(|error| cloud_error(error.to_string()))
    }
}

fn map_http_error(error: ureq::Error) -> DaemonError {
    match error {
        ureq::Error::Status(status, _) => {
            cloud_error(format!("Cloud bootstrap request failed with HTTP {status}"))
        }
        ureq::Error::Transport(error) => cloud_error(error.to_string()),
    }
}

fn cloud_error(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "managed kernel Cloud bootstrap",
        message: message.into(),
    }
}
