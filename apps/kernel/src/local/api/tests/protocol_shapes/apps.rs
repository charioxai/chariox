use crate::local::*;
use sha2::{Digest, Sha256};

#[test]
fn app_installation_protocol_shapes_are_versioned_and_preserve_generations() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 288);
    let release = AppReleaseSummary {
        version: "1.0.0".into(),
        publisher_id: "publisher".into(),
        package_digest: format!("sha256:{:064x}", 1),
        schema_version: 1,
    };
    let requests = vec![
        LocalDaemonRequest::ListAppInstallations(ListAppInstallationsRequest {
            after: None,
            limit: Some(50),
        }),
        LocalDaemonRequest::GetAppInstallation(AppInstallationRequest {
            installation_id: "todo".into(),
        }),
        LocalDaemonRequest::GetAppInstallationJournal(AppInstallationRequest {
            installation_id: "todo".into(),
        }),
    ];
    let responses = vec![
        LocalDaemonResponse::AppInstallationsListed {
            installations: vec![],
            next_cursor: None,
        },
        LocalDaemonResponse::AppInstallation {
            installation: AppInstallationSummary {
                installation_id: "todo".into(),
                app_id: "com.chariox.todo".into(),
                generation: "9223372036854775807".into(),
                active_release: Some(release.clone()),
                pending_generation: Some("9223372036854775806".into()),
                admission_paused: false,
            },
        },
        LocalDaemonResponse::AppInstallationJournal {
            installation_id: "todo".into(),
            updates: vec![AppUpdateSummary {
                base_generation: "0".into(),
                generation: "1".into(),
                release,
                phase: AppUpdatePhase::Staged,
                decision: AppCapabilityDecisionStatus::Pending,
                created_at_ms: 1,
                updated_at_ms: 2,
            }],
        },
        LocalDaemonResponse::AppRequestFailed {
            code: AppRequestErrorCode::NotFound,
        },
    ];
    for request in &requests {
        assert_eq!(
            &serde_json::from_slice::<LocalDaemonRequest>(&serde_json::to_vec(request).unwrap())
                .unwrap(),
            request
        );
    }
    for response in &responses {
        assert_eq!(
            &serde_json::from_slice::<LocalDaemonResponse>(&serde_json::to_vec(response).unwrap())
                .unwrap(),
            response
        );
    }
    let snapshot = serde_json::json!({"requests": requests, "responses": responses});
    let encoded = serde_json::to_vec(&snapshot).unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(&encoded)),
        "e147af40b512fe8437817982f8e6b8e0138586d7ea9174493608002e0e105cf6"
    );
    assert!(serde_json::from_value::<LocalDaemonRequest>(
        serde_json::json!({"ListAppInstallations": {"owner_id": "other"}})
    )
    .is_err());
    assert!(serde_json::from_value::<LocalDaemonRequest>(
        serde_json::json!({"GetAppInstallation": {"installation_id": "todo", "path": "/host"}})
    )
    .is_err());
}
