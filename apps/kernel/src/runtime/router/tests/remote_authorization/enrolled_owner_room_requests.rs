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

#[test]
fn linked_workflow_refusals_honor_mute_and_keep_read_state() {
    run_remote_authorization_large_stack_test("linked-workflow-refusal-notices", || async {
        use crate::local::{
            GetPersonInboxRequest, InboxCardChange, PersonInboxCardBody, PersonInboxOrigin,
            SetInboxCardStateRequest, SetInboxMuteRequest,
        };

        let root = crate::test_support::TestWorktree::new("linked-workflow-refusal-notices");
        let mut config = DaemonConfig::for_tests();
        config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
            user_id: ENROLLED_OWNER.into(),
            realm_id: "test-realm".into(),
            ..Default::default()
        });
        let mut app = DaemonApp::bootstrap(config).unwrap();
        let room = app
            .sessions_mut()
            .create_session(root.session_request().with_owner_user_id(ENROLLED_OWNER))
            .unwrap();
        let session_id = room.id().to_owned();
        let agent = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                CreateAgentRequest::new(&session_id, "dev-stub").with_owner_user_id(ENROLLED_OWNER),
            )
            .unwrap();
        let agent_id = agent.id().to_owned();
        let workflow = app
            .sessions_mut()
            .create_workflow(&session_id, None)
            .unwrap();
        let workflow_id = workflow.id().to_owned();
        let node = app
            .sessions_mut()
            .add_workflow_node_owned(
                &session_id,
                &workflow_id,
                &agent_id,
                ENROLLED_OWNER.into(),
                ENROLLED_OWNER.into(),
                agent_id.clone(),
            )
            .unwrap();
        // Seed endpoints whose stored owner differs from the enrolled Room owner.
        for alias in ["unmuted", "muted"] {
            let endpoint = app
                .sessions_mut()
                .create_workflow_endpoint(&session_id, &workflow_id, node.id(), Some(alias.into()))
                .unwrap();
            app.sessions_mut()
                .set_workflow_endpoint_owner(
                    &session_id,
                    &workflow_id,
                    endpoint.id(),
                    "previous-owner".into(),
                )
                .unwrap();
        }
        let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);

        let refuse = |endpoint_ref: &str| {
            let request =
                LocalDaemonRequest::InvokeWorkflowEndpoint(InvokeWorkflowEndpointRequest {
                    session_id: session_id.clone(),
                    workflow_ref: workflow_id.clone(),
                    endpoint_ref: endpoint_ref.into(),
                    prompt: Some("Start the workflow.".into()),
                    queue_ref: None,
                    publication_invocation: None,
                });
            router.dispatch(terminal_command(&request, Some(ENROLLED_OWNER)), request)
        };

        let inbox = || async {
            let LocalDaemonResponse::PersonInbox { cards, .. } = as_user(
                &router,
                ENROLLED_OWNER,
                LocalDaemonRequest::GetPersonInbox(GetPersonInboxRequest),
            )
            .await
            else {
                panic!("expected the owner's Inbox");
            };
            cards
                .into_iter()
                .map(|card| {
                    assert_eq!(card.session_id, session_id);
                    let PersonInboxCardBody::Notice(notice) = card.body else {
                        panic!("expected a workflow refusal notice");
                    };
                    assert_eq!(notice.agent_id, agent_id);
                    assert_eq!(
                        notice.origin,
                        crate::local::PersonInboxNoticeOrigin::Kernel
                    );
                    (card.card_id, notice.read, notice.body.unwrap())
                })
                .collect::<Vec<_>>()
        };

        assert!(matches!(
            refuse("unmuted").await,
            Err(DaemonError::WorkflowStartRefused { .. })
        ));
        let first = inbox().await;
        let [(card_id, false, body)] = first.as_slice() else {
            panic!("expected one unread refusal, got {first:?}");
        };
        assert!(body.ends_with("Attempts: 1"));
        as_user(
            &router,
            ENROLLED_OWNER,
            LocalDaemonRequest::SetInboxCardState(SetInboxCardStateRequest {
                card_ids: vec![card_id.clone()],
                change: InboxCardChange::Read { read: true },
            }),
        )
        .await;

        assert!(matches!(
            refuse("unmuted").await,
            Err(DaemonError::WorkflowStartRefused { .. })
        ));
        let repeated = inbox().await;
        assert_eq!(repeated.len(), 1);
        assert_eq!(repeated[0].0, *card_id);
        assert!(repeated[0].2.ends_with("Attempts: 2"));

        as_user(
            &router,
            ENROLLED_OWNER,
            LocalDaemonRequest::SetInboxMute(SetInboxMuteRequest {
                origin: PersonInboxOrigin {
                    session_id: session_id.clone(),
                    agent_id: agent_id.clone(),
                },
                muted: true,
            }),
        )
        .await;
        assert!(matches!(
            refuse("muted").await,
            Err(DaemonError::WorkflowStartRefused { .. })
        ));
        let muted = inbox().await;
        assert_eq!(muted.len(), 2);
        assert!(
            muted.iter().any(|(id, read, _)| id != card_id && *read),
            "a fresh refusal from a muted entry agent should arrive read"
        );
        assert!(
            repeated[0].1,
            "an unmuted repeat should keep the marked-read state"
        );
    });
}
