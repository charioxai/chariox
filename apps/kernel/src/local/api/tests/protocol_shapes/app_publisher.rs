use super::*;
use crate::local::*;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use sha2::{Digest, Sha256};

#[test]
fn publisher_review_wire_preserves_exact_revisions_and_excludes_client_consent() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 297);
    let requests = vec![
        LocalDaemonRequest::BeginAppPublisherEnrollment(BeginAppPublisherEnrollmentRequest {
            session_id: "session".into(),
            request_id: "retry".into(),
            publisher_id: "com.example".into(),
            key_id: "developer".into(),
            public_key_base64: STANDARD.encode([7; 32]),
            expected_revision: "9007199254740993".into(),
        }),
        LocalDaemonRequest::GetAppPublisherEnrollment(AppPublisherEnrollmentRequest {
            request_id: "retry".into(),
        }),
        LocalDaemonRequest::CancelAppPublisherEnrollment(AppPublisherEnrollmentRequest {
            request_id: "retry".into(),
        }),
    ];
    let responses = [
        AppPublisherEnrollmentPhase::Pending,
        AppPublisherEnrollmentPhase::Approved,
        AppPublisherEnrollmentPhase::Denied,
        AppPublisherEnrollmentPhase::Cancelled,
        AppPublisherEnrollmentPhase::Failed,
    ]
    .into_iter()
    .map(|phase| LocalDaemonResponse::AppPublisherEnrollmentStatus {
        operation: AppPublisherEnrollmentSummary {
            approved_revision: (phase == AppPublisherEnrollmentPhase::Approved)
                .then(|| "9007199254740994".into()),
            request_id: "retry".into(),
            phase,
            publisher_id: "com.example".into(),
            key_id: "developer".into(),
            key_fingerprint: format!("sha256:{}", "a".repeat(64)),
            interaction_id: None,
            failure: None,
        },
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
    for field in [
        "owner_id",
        "approved",
        "decision_id",
        "authority_ref",
        "deadline_ms",
        "path",
    ] {
        let mut forged = serde_json::to_value(&requests[0]).unwrap();
        forged["BeginAppPublisherEnrollment"][field] = serde_json::json!("untrusted");
        assert!(serde_json::from_value::<LocalDaemonRequest>(forged).is_err());
    }
    let snapshot = serde_json::json!({"requests":requests,"responses":responses});
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "cab71f21e31dae79978343d0014815cae131a3d38a73dd21590867ba1a7c3708"
    );
}
