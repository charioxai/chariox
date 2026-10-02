use super::*;
use crate::session::{
    ActionAdmission, CanonicalViewport, CreateSessionRequest, EnvironmentActionRequest,
    EnvironmentActionState, EnvironmentActionTerminal, EnvironmentActor, EnvironmentActorKind,
    EnvironmentLifecycle,
};

#[test]
fn room_environment_restart_retains_tabs_actions_and_idempotency_after_old_checkpoint() {
    let config = DaemonConfig::for_tests();
    let (room_id, tab_id, completed, running, request, cursor, generation) = {
        let app = DaemonApp::bootstrap(config.clone()).unwrap();
        let session = app
            .sessions_mut()
            .create_session(CreateSessionRequest::new("workspace-env", "worktree-env"))
            .unwrap();
        // This checkpoint predates every Environment mutation below.
        app.save_durable_state_snapshot().unwrap();
        let room = session.id();
        let viewport = CanonicalViewport::new(1280, 800, 1, 1280, 800).unwrap();
        app.sessions_mut()
            .start_room_environment(room, viewport)
            .unwrap();
        app.sessions_mut()
            .transition_room_environment(room, EnvironmentLifecycle::Ready)
            .unwrap();
        app.sessions_mut()
            .reconcile_room_environment_actors(
                room,
                vec![EnvironmentActor::new(
                    "human",
                    EnvironmentActorKind::Human,
                    "Human",
                )],
            )
            .unwrap();
        app.sessions_mut()
            .reconcile_room_environment_controller_tabs(
                room,
                vec![crate::session::EnvironmentTabObservation {
                    runtime_target_id: "physical-target".into(),
                    document_id: "document-1".into(),
                    url: "http://localhost/fixture".into(),
                    title: "Fixture".into(),
                }],
                Some("physical-target"),
            )
            .unwrap();
        let env = app.sessions().room_environment_snapshot(room).unwrap();
        let tab = env.tabs[0].tab_id.clone();
        let request = EnvironmentActionRequest::browser_mutation(
            "human",
            env.runtime_generation,
            "click",
            &tab,
            1,
        )
        .with_idempotency_key("completed-before-restart");
        let (
            ActionAdmission::Accepted {
                action_id: completed,
            },
            _,
        ) = app
            .sessions_mut()
            .submit_room_environment_action(room, request.clone())
            .unwrap()
        else {
            panic!("not accepted")
        };
        app.sessions_mut()
            .finish_room_environment_action(room, &completed, EnvironmentActionTerminal::Completed)
            .unwrap();
        let running_request = EnvironmentActionRequest::browser_mutation(
            "human",
            env.runtime_generation,
            "click",
            &tab,
            1,
        )
        .with_idempotency_key("interrupted-at-restart");
        let (ActionAdmission::Accepted { action_id: running }, _) = app
            .sessions_mut()
            .submit_room_environment_action(room, running_request)
            .unwrap()
        else {
            panic!("not accepted")
        };
        let last = app.sessions().room_environment_snapshot(room).unwrap();
        (
            room.to_string(),
            tab,
            completed,
            running,
            request,
            last.event_cursor,
            last.runtime_generation,
        )
    };
    let app = DaemonApp::bootstrap(config).unwrap();
    let environment = app
        .sessions()
        .room_environment_snapshot(&room_id)
        .expect("Environment restored with Room");
    assert_eq!(environment.environment_id, format!("environment-{room_id}"));
    assert_eq!(environment.tabs[0].tab_id, tab_id);
    assert!(environment.runtime_generation > generation);
    assert!(environment.event_cursor > cursor);
    let history = app
        .sessions()
        .room_environment_action_history(&room_id, None, 100)
        .unwrap();
    assert_eq!(
        history
            .actions
            .iter()
            .find(|action| action.action_id == completed)
            .unwrap()
            .state,
        EnvironmentActionState::Completed
    );
    assert_eq!(
        history
            .actions
            .iter()
            .find(|action| action.action_id == running)
            .unwrap()
            .state,
        EnvironmentActionState::Failed
    );
    assert!(
        matches!(app.sessions().existing_room_environment_action(&room_id, &request).unwrap(),
        Some(ActionAdmission::Existing { action_id, state: EnvironmentActionState::Completed }) if action_id == completed)
    );
    assert_eq!(
        app.sessions()
            .room_environment_controller_tab_binding(&room_id, &tab_id)
            .unwrap()
            .runtime_target_id,
        "physical-target"
    );
}
