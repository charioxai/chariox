// MP-08 / MP-09 / MP-10 / MP-11: the enrolled owner reaches its room, made before enrollment under
// `local`, with its Cloud identity. Room-scoped requests that compare the caller with the room's
// stored owner must treat that identity as the owner, like the sub-agent tree does.
use super::historical_workflows::terminal_command;
use super::*;
use crate::local::{ListAppInstallationsRequest, ListWorkflowNotificationsRequest};

const ENROLLED_OWNER: &str = "kernel-owner";

fn enrolled_owner_room(label: &str) -> (CommandRouter, String, crate::test_support::TestWorktree) {
    let worktree = crate::test_support::TestWorktree::new(label);
    let mut config = DaemonConfig::for_tests();
    config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
        user_id: ENROLLED_OWNER.into(),
        realm_id: "test-realm".into(),
        ..Default::default()
    });
    let app = DaemonApp::bootstrap(config).expect("daemon should boot");
    let session = app
        .sessions_mut()
        .create_session(worktree.session_request())
        .expect("session should be created");
    assert_eq!(session.owner_user_id(), DEFAULT_LOCAL_USER_ID);
    let session_id = session.id().to_string();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    (router, session_id, worktree)
}

async fn as_user(
    router: &CommandRouter,
    user: &str,
    request: LocalDaemonRequest,
) -> LocalDaemonResponse {
    router
        .dispatch(terminal_command(&request, Some(user)), request)
        .await
        .expect("the enrolled owner is admitted to its room")
}

#[test]
fn the_enrolled_owner_lists_the_workflow_notifications_of_a_room_made_before_enrollment() {
    run_remote_authorization_large_stack_test("enrolled-owner-notifications", || async {
        let (router, session_id, _worktree) = enrolled_owner_room("enrolled-owner-notifications");
        let list = || {
            LocalDaemonRequest::ListWorkflowNotifications(ListWorkflowNotificationsRequest {
                session_id: session_id.clone(),
            })
        };
        let as_enrolled = as_user(&router, ENROLLED_OWNER, list()).await;
        assert!(
            matches!(
                as_enrolled,
                LocalDaemonResponse::WorkflowNotifications { .. }
            ),
            "{as_enrolled:?}"
        );
    });
}

#[test]
fn the_enrolled_owner_lists_the_apps_of_a_room_made_before_enrollment() {
    run_remote_authorization_large_stack_test("enrolled-owner-apps", || async {
        let (router, session_id, _worktree) = enrolled_owner_room("enrolled-owner-apps");
        let list = || {
            LocalDaemonRequest::ListAppInstallations(ListAppInstallationsRequest {
                session_id: Some(session_id.clone()),
                after: None,
                limit: None,
            })
        };
        let as_local = as_user(&router, DEFAULT_LOCAL_USER_ID, list()).await;
        assert!(
            matches!(as_local, LocalDaemonResponse::AppInstallationsListed { .. }),
            "{as_local:?}"
        );
        let as_enrolled = as_user(&router, ENROLLED_OWNER, list()).await;
        assert!(
            matches!(
                as_enrolled,
                LocalDaemonResponse::AppInstallationsListed { .. }
            ),
            "{as_enrolled:?}"
        );
    });
}
