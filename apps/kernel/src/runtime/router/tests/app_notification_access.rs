//! MP-08 / MP-09 / MP-10 / MP-11: authorization through the real router.
use super::*;
use crate::{
    app::DaemonApp,
    config::DaemonConfig,
    local::{AppRequestErrorCode, ConfigureAppAutomationRequest, ListAppInstallationsRequest},
};

fn workflow(
    app: &mut DaemonApp,
    root: &std::path::Path,
    label: &str,
    creator: &str,
) -> (String, String) {
    let path = root.to_str().unwrap();
    let (session, _) = crate::app::KernelSessionService::new(app)
        .create_session(crate::session::CreateSessionRequest::new(path, path))
        .unwrap();
    let agent = crate::app::KernelSessionService::new(app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "dev-stub").with_alias(label),
        )
        .unwrap();
    let workflow = app
        .sessions_mut()
        .create_workflow(session.id(), Some(label.into()))
        .unwrap();
    let node = app
        .sessions_mut()
        .add_workflow_node(session.id(), workflow.id(), agent.id())
        .unwrap();
    let endpoint = app
        .sessions_mut()
        .create_workflow_endpoint(session.id(), workflow.id(), node.id(), Some("entry".into()))
        .unwrap();
    let publication = app
        .sessions_mut()
        .create_workflow_publication_idempotent(
            session.id(),
            workflow.id(),
            endpoint.id(),
            None,
            None,
            Some("default".into()),
            Some(label.into()),
            Some("event_based".into()),
            None,
            vec![],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            vec![agent],
            creator.into(),
        )
        .unwrap();
    app.sessions_mut()
        .ensure_primary_workflow_runtime_instance(session.id())
        .unwrap();
    app.durable_state_store()
        .persist_workflow_runtime_transition(
            &app.sessions().get_session(session.id()).unwrap(),
            "evval_fixture",
        )
        .unwrap();
    (session.id().into(), publication.id().into())
}
async fn request(
    router: &CommandRouter,
    caller: &str,
    request: LocalDaemonRequest,
) -> LocalDaemonResponse {
    let mut command = KernelCommand::from_local_request(
        format!("notification-test-{}", rand::random::<u64>()),
        None,
        None,
        &request,
    );
    command.caller.user_id = Some(caller.into());
    router.dispatch(command, request).await.unwrap()
}

#[tokio::test]
async fn scoped_app_listing_reaches_the_room_owner_through_router() {
    let root = crate::test_support::TestWorktree::new("app-list-router");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let (room, _) = workflow(&mut app, root.path(), "owner", "local");
    let catalog = crate::durable_state::app_state::fixture_room_event_catalog(
        &app.durable_state_store(),
        "local",
    );
    let mut current = app.sessions().get_session(&room).unwrap();
    current.add_member(
        "guest",
        Some("local".into()),
        crate::session::CollaborationLevel::Full,
    );
    app.sessions_mut().restore_session(current);
    let agent = app.agents().get_session_agents(&room)[0].clone();
    app.agents_mut()
        .grant_extension(
            agent.id(),
            crate::extension::ExtensionGrant::app(catalog.installation_id()),
        )
        .unwrap();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 2);
    let list = || {
        LocalDaemonRequest::ListAppInstallations(ListAppInstallationsRequest {
            session_id: Some(room.clone()),
            after: None,
            limit: None,
        })
    };
    let LocalDaemonResponse::AppInstallationsListed { installations, .. } =
        request(&router, "guest", list()).await
    else {
        panic!("scoped listing")
    };
    assert_eq!(installations.len(), 1, "guest sees the bound owner App");
    assert_eq!(installations[0].events[0].name, "changed");
    assert!(matches!(
        request(&router, "outsider", list()).await,
        LocalDaemonResponse::AppRequestFailed {
            code: AppRequestErrorCode::Unauthorized
        }
    ));
}

#[tokio::test]
async fn guest_cannot_replace_another_members_automation_through_router() {
    let root = crate::test_support::TestWorktree::new("app-config-router");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let (room, owner_publication) = workflow(&mut app, root.path(), "owner", "local");
    let mut current = app.sessions().get_session(&room).unwrap();
    current.add_member(
        "guest",
        Some("local".into()),
        crate::session::CollaborationLevel::Full,
    );
    app.sessions_mut().restore_session(current);
    let catalog = crate::durable_state::app_state::fixture_room_event_catalog(
        &app.durable_state_store(),
        "local",
    );
    let agent = app.agents().get_session_agents(&room)[0].clone();
    app.agents_mut()
        .grant_extension(
            agent.id(),
            crate::extension::ExtensionGrant::app(catalog.installation_id()),
        )
        .unwrap();
    let workflow = app.sessions().get_session(&room).unwrap().workflows()[0].clone();
    let guest_endpoint = app
        .sessions_mut()
        .create_workflow_endpoint(
            &room,
            workflow.id(),
            workflow.nodes()[0].id(),
            Some("guest-entry".into()),
        )
        .unwrap();
    // Endpoint ownership and publication creation are the same normal kernel identities.
    let mut current = app.sessions().get_session(&room).unwrap();
    current
        .workflow_mut(workflow.id())
        .unwrap()
        .endpoint_mut(guest_endpoint.id())
        .unwrap()
        .set_owner_user_id("guest");
    app.sessions_mut().restore_session(current);
    let publication_agents = app.agents().get_session_agents(&room);
    let publication = app
        .sessions_mut()
        .create_workflow_publication_idempotent(
            &room,
            workflow.id(),
            guest_endpoint.id(),
            None,
            None,
            Some("default".into()),
            Some("guest-publication".into()),
            Some("event_based".into()),
            None,
            vec![],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            publication_agents,
            "guest".into(),
        )
        .unwrap();
    app.durable_state_store()
        .persist_workflow_runtime_transition(
            &app.sessions().get_session(&room).unwrap(),
            "guest publication",
        )
        .unwrap();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 2);
    let configure = |publication: &str, revision| {
        LocalDaemonRequest::ConfigureAppAutomation(ConfigureAppAutomationRequest {
            installation_id: catalog.installation_id().into(),
            automation_id: "owner-binding".into(),
            expected_revision: revision,
            event_name: "changed".into(),
            session_id: room.clone(),
            publication_ref: publication.into(),
            queue_ref: None,
            scheduled: false,
        })
    };
    assert!(matches!(
        request(&router, "local", configure(&owner_publication, 0)).await,
        LocalDaemonResponse::AppAutomation { .. }
    ));
    assert!(
        matches!(
            request(&router, "guest", configure(publication.id(), 1)).await,
            LocalDaemonResponse::AppRequestFailed {
                code: AppRequestErrorCode::Unauthorized
            }
        ),
        "a guest must authorize the existing binding as well as the proposed one"
    );
}
