use super::*;
use crate::local::*;
use sha2::{Digest, Sha256};

#[test]
fn install_operation_protocol_shapes_preserve_owner_decision_and_immediate_ack() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 296);
    let requests = vec![
        LocalDaemonRequest::BeginAppInstall(BeginAppInstallRequest {
            session_id: "session".into(),
            request_id: "retry".into(),
            upload_handle: format!("upload_{}", "a".repeat(64)),
            expected_package_digest: format!("sha256:{}", "b".repeat(64)),
        }),
        LocalDaemonRequest::GetAppInstallOperation(AppInstallOperationRequest {
            request_id: "retry".into(),
        }),
        LocalDaemonRequest::CancelAppInstallOperation(AppInstallOperationRequest {
            request_id: "retry".into(),
        }),
    ];
    let responses = vec![
        AppInstallOperationPhase::Preparing,
        AppInstallOperationPhase::AwaitingApproval,
        AppInstallOperationPhase::Starting,
        AppInstallOperationPhase::Committed,
        AppInstallOperationPhase::Cancelled,
        AppInstallOperationPhase::Failed,
    ]
    .into_iter()
    .map(|phase| {
        let preparing = phase == AppInstallOperationPhase::Preparing;
        LocalDaemonResponse::AppInstallOperationStatus {
            operation: AppInstallOperationSummary {
                request_id: "retry".into(),
                phase,
                installation_id: (!preparing).then(|| "app_opaque".into()),
                generation: (!preparing).then(|| "1".into()),
                package_digest: format!("sha256:{}", "b".repeat(64)),
                interaction_id: None,
                failure: None,
            },
        }
    })
    .collect::<Vec<_>>();
    for request in &requests {
        assert_eq!(
            *request,
            serde_json::from_value::<LocalDaemonRequest>(serde_json::to_value(request).unwrap())
                .unwrap()
        );
    }
    for response in &responses {
        assert_eq!(
            *response,
            serde_json::from_value::<LocalDaemonResponse>(serde_json::to_value(response).unwrap())
                .unwrap()
        );
    }
    let snapshot = serde_json::json!({"requests":requests,"responses":responses});
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "44cf902e0e4d510b7525f148621c96bdcb8f2af6461a750ab81d17af7008034e"
    );
    for field in [
        "owner_id",
        "publisher_key",
        "capability_approval",
        "approved",
        "deadline_ms",
        "path",
    ] {
        let mut forged = serde_json::to_value(&requests[0]).unwrap();
        forged["BeginAppInstall"][field] = serde_json::json!("untrusted");
        assert!(serde_json::from_value::<LocalDaemonRequest>(forged).is_err());
    }
}
