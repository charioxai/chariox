use std::io::Read;
use std::time::Duration;

use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::error::DaemonError;

use super::context_plan::ManagedKernelContextPlan;
use super::freshness::{ManagedKernelFreshnessEvidence, ManagedKernelRuntimeIdentityReport};

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
    #[serde(default)]
    pub(super) managed_repository_root: Option<String>,
    pub(super) context_plan: ManagedKernelContextPlan,
    pub(super) cloud_relay: ManagedCloudRelayProfile,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) freshness_evidence: Option<ManagedKernelFreshnessEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ConfirmResponse {
    pub(super) confirmed: bool,
    pub(super) observed_state: String,
    #[serde(default)]
    pub(super) managed_repository_root: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct RuntimeIdentityReportResponse {
    pub(super) accepted: bool,
    pub(super) environment_id: String,
    pub(super) generation: u64,
    pub(super) observed_at: String,
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
    fn report_runtime_identity(
        &self,
        api_url: &str,
        request: &ManagedKernelRuntimeIdentityReport,
    ) -> Result<RuntimeIdentityReportResponse, DaemonError>;
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

    fn confirm(
        &self,
        api_url: &str,
        request: &ConfirmRequest,
    ) -> Result<ConfirmResponse, DaemonError> {
        self.post_managed(api_url, "/v1/managed-kernels/bootstrap/confirm", request)
    }

    fn report_runtime_identity(
        &self,
        api_url: &str,
        request: &ManagedKernelRuntimeIdentityReport,
    ) -> Result<RuntimeIdentityReportResponse, DaemonError> {
        self.post_managed(
            api_url,
            "/v1/managed-kernels/reimage/runtime-identity",
            request,
        )
    }
}

impl HttpBootstrapCloudClient {
    pub(super) fn post_managed<T: DeserializeOwned>(
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        ConfirmRequest, ManagedKernelFreshnessEvidence, ManagedKernelRuntimeIdentityReport,
    };
    use crate::managed_bootstrap::freshness::ManagedKernelResidueChecks;

    #[test]
    fn confirm_and_pre_reimage_report_wires_match_cloud_contract() {
        let evidence = ManagedKernelFreshnessEvidence {
            linux_boot_id: "01234567-89ab-cdef-0123-456789abcdef".to_string(),
            os_machine_id: "a".repeat(32),
            runtime_release_digest: format!("sha256:{}", "b".repeat(64)),
            runtime_source_commit: "c".repeat(40),
            runtime_source_tree: "d".repeat(40),
            residue_checks: ManagedKernelResidueChecks {
                old_services_absent: true,
                old_processes_absent: true,
                old_state_absent: true,
            },
        };
        let confirm = ConfirmRequest {
            token: format!("mkboot_{}", "t".repeat(40)),
            environment_id: "environment-1".to_string(),
            machine_id: "machine-1".to_string(),
            machine_credential: format!("mcred_{}", "x".repeat(40)),
            freshness_evidence: Some(evidence),
        };
        assert_eq!(
            serde_json::to_value(confirm).expect("confirm serializes"),
            json!({
                "token": format!("mkboot_{}", "t".repeat(40)),
                "environmentId": "environment-1",
                "machineId": "machine-1",
                "machineCredential": format!("mcred_{}", "x".repeat(40)),
                "freshnessEvidence": {
                    "linuxBootId": "01234567-89ab-cdef-0123-456789abcdef",
                    "osMachineId": "a".repeat(32),
                    "runtimeReleaseDigest": format!("sha256:{}", "b".repeat(64)),
                    "runtimeSourceCommit": "c".repeat(40),
                    "runtimeSourceTree": "d".repeat(40),
                    "residueChecks": {
                        "oldServicesAbsent": true,
                        "oldProcessesAbsent": true,
                        "oldStateAbsent": true,
                    },
                },
            })
        );

        let report = ManagedKernelRuntimeIdentityReport {
            environment_id: "environment-1".to_string(),
            machine_id: "machine-1".to_string(),
            kernel_id: "kernel-1".to_string(),
            generation: 4,
            machine_credential: format!("mcred_{}", "x".repeat(40)),
            linux_boot_id: "01234567-89ab-cdef-0123-456789abcdef".to_string(),
            os_machine_id: "a".repeat(32),
            runtime_release_digest: format!("sha256:{}", "b".repeat(64)),
            runtime_source_commit: "c".repeat(40),
            runtime_source_tree: "d".repeat(40),
            observed_at: "2026-09-22T01:02:03.000Z".to_string(),
        };
        assert_eq!(
            serde_json::to_value(report).expect("runtime identity report serializes"),
            json!({
                "environmentId": "environment-1",
                "machineId": "machine-1",
                "kernelId": "kernel-1",
                "generation": 4,
                "machineCredential": format!("mcred_{}", "x".repeat(40)),
                "linuxBootId": "01234567-89ab-cdef-0123-456789abcdef",
                "osMachineId": "a".repeat(32),
                "runtimeReleaseDigest": format!("sha256:{}", "b".repeat(64)),
                "runtimeSourceCommit": "c".repeat(40),
                "runtimeSourceTree": "d".repeat(40),
                "observedAt": "2026-09-22T01:02:03.000Z",
            })
        );
    }
}
