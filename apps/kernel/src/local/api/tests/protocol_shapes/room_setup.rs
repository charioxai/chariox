//! The wire shapes of Room setup and the Room name suggestion (local 1004).
use super::*;
use crate::session::{
    RoomSetupRequest, RoomSetupState, RoomSetupStep, RoomSetupStepId, RoomSetupStepState,
};

fn json(value: impl serde::Serialize) -> serde_json::Value {
    serde_json::to_value(value).unwrap()
}

#[test]
fn create_session_setup_wire_shape() {
    let request = crate::session::CreateSessionRequest::new("/work/app", "/work/app").with_setup(
        RoomSetupRequest {
            git_init: true,
            new_worktree: true,
            publish: true,
        },
    );
    assert_eq!(
        json(&request)["setup"],
        serde_json::json!({
            "git_init": true,
            "new_worktree": true,
            "publish": true
        })
    );
    let plain = crate::session::CreateSessionRequest::new("/work/app", "/work/app");
    assert!(json(&plain).get("setup").is_none());

    // Absent flags are off.
    let parsed: RoomSetupRequest =
        serde_json::from_value(serde_json::json!({"publish": true})).unwrap();
    assert!(!parsed.git_init && !parsed.new_worktree && parsed.publish);
    let none: RoomSetupRequest = serde_json::from_value(serde_json::json!({})).unwrap();
    assert!(!none.publish);
}

#[test]
fn room_setup_projection_wire_shape() {
    let state = RoomSetupState {
        steps: vec![
            RoomSetupStep {
                id: RoomSetupStepId::GitInit,
                state: RoomSetupStepState::Done,
                message: None,
            },
            RoomSetupStep {
                id: RoomSetupStepId::Worktree,
                state: RoomSetupStepState::Running,
                message: None,
            },
            RoomSetupStep {
                id: RoomSetupStepId::Publish,
                state: RoomSetupStepState::Failed,
                message: Some("not logged in; run `gh auth login`".into()),
            },
        ],
    };
    assert_eq!(
        json(&state),
        serde_json::json!({"steps": [
            {"id": "git_init", "state": "done"},
            {"id": "worktree", "state": "running"},
            {"id": "publish", "state": "failed", "message": "not logged in; run `gh auth login`"}
        ]})
    );
    let mut session = crate::session::RuntimeSession::new("room", None, "/w", "/w", "m", "k");
    assert!(json(&session).get("room_setup").is_none());
    session.set_room_setup(Some(state.clone()));
    assert_eq!(json(&session)["room_setup"], json(&state));
}

#[test]
fn room_setup_requests_and_responses_wire_shape() {
    assert_eq!(
        json(LocalDaemonRequest::SuggestSessionAlias(
            crate::local::SuggestSessionAliasRequest {
                workspace_id: "/work/app".into()
            }
        )),
        serde_json::json!({"SuggestSessionAlias": {"workspace_id": "/work/app"}})
    );
    assert_eq!(
        json(LocalDaemonRequest::RetryRoomSetup(
            crate::local::RetryRoomSetupRequest {
                session_id: "s-1".into()
            }
        )),
        serde_json::json!({"RetryRoomSetup": {"session_id": "s-1"}})
    );
    assert_eq!(
        json(LocalDaemonRequest::DismissRoomSetup(
            crate::local::DismissRoomSetupRequest {
                session_id: "s-1".into()
            }
        )),
        serde_json::json!({"DismissRoomSetup": {"session_id": "s-1"}})
    );
    assert_eq!(
        json(LocalDaemonResponse::SessionAliasSuggested {
            workspace_id: "/work/app".into(),
            alias: "app-2".into()
        }),
        serde_json::json!({"SessionAliasSuggested": {"workspace_id": "/work/app", "alias": "app-2"}})
    );
    let session = crate::session::RuntimeSession::new("s-1", None, "/w", "/w", "m", "k");
    let updated = json(LocalDaemonResponse::RoomSetupUpdated {
        session: session.clone(),
    });
    assert_eq!(updated["RoomSetupUpdated"]["session"], json(&session));
}
