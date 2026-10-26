//! MP-08 / MP-11: protocol-491 kernels own enrollment, not human Cloud sessions.
//! Keep old wire variants recognizable so clients receive an actionable error.
use crate::{error::DaemonError, local::LocalDaemonRequest};

pub(super) fn require_kernel_authority(request: &LocalDaemonRequest) -> Result<(), DaemonError> {
    if matches!(
        request,
        LocalDaemonRequest::PairCloudRelayClient(_)
            | LocalDaemonRequest::PairCloudRelayMachine(_)
            | LocalDaemonRequest::CreateCloudSessionInvite(_)
            | LocalDaemonRequest::ShowCloudSessionInvite(_)
            | LocalDaemonRequest::AcceptCloudSessionInvite(_)
            | LocalDaemonRequest::RevokeCloudSessionInvite(_)
            | LocalDaemonRequest::ListCloudSessionMembers(_)
            | LocalDaemonRequest::ListCloudCollaborators(_)
            | LocalDaemonRequest::ListManagedEnvironmentCatalog(_)
            | LocalDaemonRequest::CreateDisposableWorker(_)
            | LocalDaemonRequest::GetDisposableWorker(_)
            | LocalDaemonRequest::ReleaseDisposableWorker(_)
            | LocalDaemonRequest::KeepDisposableWorkerRunning(_)
            | LocalDaemonRequest::PrepareDisposableWorkerContextTransfer(_)
            | LocalDaemonRequest::KeepManagedEnvironmentRunning(_)
            | LocalDaemonRequest::GetManagedEnvironment(_)
            | LocalDaemonRequest::GetManagedEnvironmentReimagePreflight(_)
            | LocalDaemonRequest::GetManagedEnvironmentReimageReceipt(_)
            | LocalDaemonRequest::PrepareManagedEnvironmentContextTransfer(_)
            | LocalDaemonRequest::PrepareManagedEnvironmentGitCredentialEnrollment(_)
            | LocalDaemonRequest::CreateManagedEnvironment(_)
            | LocalDaemonRequest::RequestManagedEnvironmentLifecycle(_)
            | LocalDaemonRequest::RequestManagedEnvironmentReimage(_)
            | LocalDaemonRequest::RequestManagedEnvironmentReleaseUpdate(_)
            | LocalDaemonRequest::GetManagedEnvironmentReleaseUpdate(_)
    ) {
        return Err(DaemonError::RelayTransport {
            operation: "retired kernel human Cloud control",
            code: "client_cloud_authority_required".into(),
            retryable: false,
            message: "This protocol-491 operation belongs to the authenticated client Cloud account. Use the client Cloud controls; /relay cloud pair is retired in favor of terminal pairing and independent /cloud link enrollment.".into(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mp08_mp11_retired_human_controls_have_an_actionable_protocol_boundary() {
        for name in ["ListCloudCollaborators", "ListManagedEnvironmentCatalog"] {
            let request: LocalDaemonRequest =
                serde_json::from_value(serde_json::json!({name:null})).unwrap();
            let error = require_kernel_authority(&request).unwrap_err();
            assert!(
                matches!(error, DaemonError::RelayTransport {ref code, retryable:false,..} if code == "client_cloud_authority_required")
            );
        }
        let status: LocalDaemonRequest =
            serde_json::from_value(serde_json::json!({"CloudRelayStatus":null})).unwrap();
        assert!(require_kernel_authority(&status).is_ok());
        let runtime: LocalDaemonRequest = serde_json::from_value(
            serde_json::json!({"GetManagedContextTransferStatus":{"contextId":"test"}}),
        )
        .unwrap();
        assert!(require_kernel_authority(&runtime).is_ok());
    }
}
