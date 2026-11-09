//! MP-08 / MP-09 / MP-10 / MP-11: the enrolled owner previews and consents the Apps of a
//! publication its room made before enrollment under `local`. Another user still cannot.
use super::*;
use crate::local::{
    AppRequestErrorCode, PrepareDeploymentAppsRequest, PreviewDeploymentAppsRequest,
};

const ENROLLED_OWNER: &str = "kernel-owner";

fn failed(
    response: Result<LocalDaemonResponse, crate::DaemonError>,
) -> Option<AppRequestErrorCode> {
    match response.unwrap() {
        LocalDaemonResponse::AppRequestFailed { code } => Some(code),
        LocalDaemonResponse::DeploymentAppsPreview { .. } => None,
        response => panic!("unexpected response: {response:?}"),
    }
}

#[test]
fn the_enrolled_owner_previews_and_consents_a_publication_made_before_enrollment() {
    let root = temp_root("enrolled-owner");
    let mut config = crate::DaemonConfig::for_tests();
    config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
        user_id: ENROLLED_OWNER.into(),
        ..Default::default()
    });
    let harness = harness_with_app_config(&root, config);
    let graph = create_publication_test_graph(&harness, "app-enrolled-owner");
    let publication = publish(&harness, &graph, "todo-enrolled", "ingress");
    assert_eq!(publication.created_by_user_id(), DEFAULT_LOCAL_USER_ID);

    let preview = |user: &str| {
        failed(harness.dispatch_as_user(
            user,
            LocalDaemonRequest::PreviewDeploymentApps(PreviewDeploymentAppsRequest {
                session_id: graph.session_id.clone(),
                publication_ref: publication.id().into(),
                package_digest: None,
            }),
        ))
    };
    // Without a recorded plan the consent reaches `InvalidRequest`, past the owner check.
    let consent = |user: &str| {
        failed(harness.dispatch_as_user(
            user,
            LocalDaemonRequest::PrepareDeploymentApps(PrepareDeploymentAppsRequest {
                session_id: graph.session_id.clone(),
                request_id: "request-1".into(),
                publication_ref: publication.id().into(),
                deployment_id: "deployment-1".into(),
                release_id: "release-1".into(),
                package_digest: format!("sha256:{}", "a".repeat(64)),
            }),
        ))
    };
    assert_eq!(preview(ENROLLED_OWNER), None);
    assert_eq!(
        consent(ENROLLED_OWNER),
        Some(AppRequestErrorCode::InvalidRequest)
    );
    assert_eq!(preview("stranger"), Some(AppRequestErrorCode::Unauthorized));
    assert_eq!(consent("stranger"), Some(AppRequestErrorCode::Unauthorized));
    drop(harness);
    let _ = std::fs::remove_dir_all(root);
}
