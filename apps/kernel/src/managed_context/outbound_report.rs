//! Tells Cloud why a machine's context copy stopped, so the machine's row can say it. Cloud gets a reason from a closed set; the kernel log keeps the full error.
use serde_json::json;

use super::{
    ManagedContextOutboundOperationPhase, ManagedContextOutboundOperationStatus,
    ManagedContextOutboundOperationStore, ManagedContextTransferTicket,
};
use crate::config::DaemonConfig;
use crate::error::DaemonError;
use crate::runtime::cloud_api_client::post_cloud_json;
use crate::runtime::terminal_pairings::public_key_thumbprint;

const REPORT_ENDPOINT: &str = "/v1/kernels/context/report";

/// What stopped the copy, as Cloud's machine row words it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CopyBlock {
    NeedsVault,
    SourceUnreachable,
    NotCopyable,
    Stopped,
}

impl CopyBlock {
    pub(super) fn of(status: &ManagedContextOutboundOperationStatus) -> Self {
        match status.failure_code.as_deref() {
            Some("target_vault_locked") => Self::NeedsVault,
            Some("managed_context_source_unavailable") => Self::SourceUnreachable,
            _ if !status.retryable => Self::NotCopyable,
            _ => Self::Stopped,
        }
    }

    fn reason(self) -> &'static str {
        match self {
            Self::NeedsVault => "needs_vault",
            Self::SourceUnreachable => "source_unreachable",
            Self::NotCopyable => "not_copyable",
            Self::Stopped => "stopped",
        }
    }
}

/// The request body that proves this kernel is the managed environment's context source.
pub(super) fn source_identity_body(
    config: &DaemonConfig,
    environment_id: &str,
) -> Result<serde_json::Value, DaemonError> {
    let cloud = config.cloud_relay.as_ref().ok_or_else(|| {
        super::outbound_service_error("source kernel is not connected to Chariox Cloud", true)
    })?;
    let machine_id = cloud.machine_id.as_deref().ok_or_else(|| {
        super::outbound_service_error("source kernel Cloud Machine identity is unavailable", true)
    })?;
    let mut body = json!({
        "accountId": cloud.account_id,
        "environmentId": environment_id,
        "machineId": machine_id,
        "kernelId": config.daemon_id,
        "relayRealmId": cloud.realm_id,
        "keyThumbprint": public_key_thumbprint(&config.relay_public_key),
    });
    if let Some(credential) = cloud.kernel_credential.as_deref() {
        body["kernelCredential"] = json!(credential);
    } else if let Some(credential) = cloud.machine_credential.as_deref() {
        // Only legacy/managed-worker profiles retain Machine authority.
        body["machineCredential"] = json!(credential);
    } else {
        return Err(super::outbound_service_error(
            "source kernel Cloud credential is unavailable",
            true,
        ));
    }
    Ok(body)
}

/// A managed machine's copy, remembered before it runs so its outcome can be reported after.
pub(super) struct ReportTarget {
    config: DaemonConfig,
    store: ManagedContextOutboundOperationStore,
    environment_id: String,
    context_id: String,
    managed_environment: bool,
}

impl ReportTarget {
    pub(super) fn new(
        config: &DaemonConfig,
        store: &ManagedContextOutboundOperationStore,
        ticket: &ManagedContextTransferTicket,
    ) -> Self {
        let plan = ticket.context_plan.package_binding();
        Self {
            config: config.clone(),
            store: store.clone(),
            environment_id: ticket.environment_id.clone(),
            context_id: plan.context_id,
            // An owner copy has no machine row in Cloud to report to.
            managed_environment: plan.destination.is_none(),
        }
    }

    /// Reports a failed copy to Cloud; a report that cannot be sent is logged, never the copy's failure.
    pub(super) async fn report_when_failed(self) {
        let Some(status) = self.store.get(&self.context_id) else {
            return;
        };
        if !self.managed_environment || status.phase != ManagedContextOutboundOperationPhase::Failed
        {
            return;
        }
        let block = CopyBlock::of(&status);
        tracing::warn!(
            context_id = %self.context_id,
            environment_id = %self.environment_id,
            code = status.failure_code.as_deref().unwrap_or("unknown"),
            message = status.failure_message.as_deref().unwrap_or(""),
            retryable = status.retryable,
            reason = block.reason(),
            "managed context copy stopped"
        );
        if let Err(error) = self.send(block).await {
            tracing::warn!(context_id = %self.context_id, %error, "Cloud was not told why the managed context copy stopped");
        }
    }

    async fn send(&self, block: CopyBlock) -> Result<(), DaemonError> {
        let mut body = source_identity_body(&self.config, &self.environment_id)?;
        body["contextId"] = json!(self.context_id);
        body["reason"] = json!(block.reason());
        let cloud = self.config.cloud_relay.as_ref().ok_or_else(|| {
            super::outbound_service_error("source kernel is not connected to Chariox Cloud", true)
        })?;
        post_cloud_json::<serde_json::Value>(cloud.api_url.clone(), REPORT_ENDPOINT, body)
            .await
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::super::ManagedContextTransferTarget;
    use super::*;
    use crate::config::PersistedCloudRelayProfile;
    use crate::managed_bootstrap::ManagedKernelContextPlan;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn failed(code: Option<&str>, retryable: bool) -> ManagedContextOutboundOperationStatus {
        ManagedContextOutboundOperationStatus {
            context_id: "context-1".into(),
            plan_digest: "sha256:one".into(),
            phase: ManagedContextOutboundOperationPhase::Failed,
            accepted_bytes: 0,
            package_size_bytes: 0,
            receipt: None,
            failure_code: code.map(str::to_string),
            failure_message: Some("repository /srv/x contains Git LFS pointers".into()),
            retryable,
            updated_at_ms: 0,
        }
    }

    #[test]
    fn a_failure_is_worded_by_what_the_person_can_do_about_it() {
        let reason = |code, retryable| CopyBlock::of(&failed(code, retryable)).reason();
        assert_eq!(reason(Some("target_vault_locked"), true), "needs_vault");
        assert_eq!(
            reason(Some("managed_context_source_unavailable"), true),
            "source_unreachable"
        );
        assert_eq!(
            reason(Some("invalid_managed_context"), false),
            "not_copyable"
        );
        assert_eq!(reason(Some("target_import_failed"), true), "stopped");
        assert_eq!(reason(None, true), "stopped");
    }

    #[tokio::test]
    async fn a_failed_copy_tells_cloud_its_reason_and_never_its_error_text() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut config = DaemonConfig::for_tests();
        config.cloud_relay = Some(PersistedCloudRelayProfile {
            api_url: format!("http://{}", listener.local_addr().unwrap()),
            account_id: "account-1".into(),
            realm_id: "realm-1".into(),
            machine_id: Some(config.host_machine_id.clone()),
            kernel_credential: Some("synthetic-kernel-only-report-credential".into()),
            ..Default::default()
        });
        let ticket = ManagedContextTransferTicket {
            environment_id: "environment-1".to_string(),
            context_plan: ManagedKernelContextPlan::source_project_for_tests(
                "context-1",
                "realm-1",
                &config.daemon_id,
                &public_key_thumbprint(&config.relay_public_key),
                "project-1",
            ),
            target: ManagedContextTransferTarget {
                relay_realm_id: "realm-1".to_string(),
                machine_id: "target-machine".to_string(),
                kernel_id: "target-kernel".to_string(),
                key_thumbprint: public_key_thumbprint("target-key"),
                relay_public_key: "target-key".to_string(),
            },
        };
        let store = ManagedContextOutboundOperationStore::default();
        store
            .start(
                "context-1",
                &ticket.context_plan.package_binding().plan_digest,
                false,
            )
            .unwrap();
        store.update("context-1", |status| {
            *status = failed(Some("invalid_managed_context"), false)
        });
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let (headers, body) = loop {
                let mut buf = [0; 4096];
                let size = stream.read(&mut buf).await.unwrap();
                assert!(size > 0);
                request.extend_from_slice(&buf[..size]);
                let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") else {
                    continue;
                };
                let headers = String::from_utf8_lossy(&request[..end]).to_string();
                let len: usize = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|value| value.trim().parse().unwrap())
                    })
                    .unwrap_or(0);
                if request.len() >= end + 4 + len {
                    break (headers, request[end + 4..end + 4 + len].to_vec());
                }
            };
            let response = b"{\"recorded\":true}";
            let header = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", response.len());
            stream.write_all(header.as_bytes()).await.unwrap();
            stream.write_all(response).await.unwrap();
            (headers, String::from_utf8(body).unwrap())
        });
        ReportTarget::new(&config, &store, &ticket)
            .report_when_failed()
            .await;
        let (headers, body) = server.await.unwrap();
        assert!(
            headers.starts_with("POST /v1/kernels/context/report "),
            "{headers}"
        );
        let body: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(body["environmentId"], "environment-1");
        assert_eq!(body["contextId"], "context-1");
        assert_eq!(body["reason"], "not_copyable");
        assert!(body.get("kernelCredential").is_some());
        assert!(
            !body.to_string().contains("LFS"),
            "the error text stays in the kernel log"
        );
    }
}
