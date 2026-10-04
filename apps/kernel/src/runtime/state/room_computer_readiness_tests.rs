//! MP-08/MP-10/MP-11: Computer admission after restored controller health.

use super::browser_controller_action_execution_runtime_state::computer_input_reconcile_test_support::{
    install_screen_tool, TestRoom, TestTools,
};
use crate::session::{EnvironmentComponent, EnvironmentComponentHealthState, EnvironmentLifecycle};
use crate::transport::room_browser_controller::RoomComputerInputAction;

fn degrade(room: &TestRoom) {
    for component in [
        EnvironmentComponent::BrowserController,
        EnvironmentComponent::Browser,
    ] {
        room.runtime
            .update_room_environment_component_health(
                &room.session_id,
                component,
                EnvironmentComponentHealthState::Unavailable,
                Some("kernel_restarted"),
            )
            .unwrap();
    }
    room.runtime
        .transition_room_environment(&room.session_id, EnvironmentLifecycle::Degraded)
        .unwrap();
}

#[tokio::test]
async fn agent_computer_input_reconciles_existing_degraded_room_before_admission() {
    let tools = TestTools::new("computer-restored-room");
    let _screen_tool = install_screen_tool(&tools.screen_tool);
    let mut room = TestRoom::new("computer-restored-room");
    room.enable_browser_controller(&tools).await;
    degrade(&room);
    let result = room
        .runtime
        .execute_computer_input_as_agent(
            &room.session_id,
            &room.agent_id,
            RoomComputerInputAction::PointerMove { x: 10, y: 10 },
        )
        .await;
    let environment = room
        .runtime
        .room_environment_snapshot(&room.session_id)
        .unwrap();
    room.stop_browser_controller().await;
    // This local fixture has no verified headed-slice desktop/streamer. Recover
    // Browser health, but retain Computer admission's existing readiness fence.
    assert!(result.is_err());
    assert_eq!(environment.lifecycle, EnvironmentLifecycle::Degraded);
    for component in [
        EnvironmentComponent::BrowserController,
        EnvironmentComponent::Browser,
    ] {
        assert_eq!(
            environment
                .health
                .iter()
                .find(|health| health.component == component)
                .unwrap()
                .state,
            EnvironmentComponentHealthState::Ready
        );
    }
    assert_eq!(environment.tabs[0].title, "After input");
}

#[tokio::test]
async fn computer_input_does_not_restart_stopped_room() {
    let tools = TestTools::new("computer-stopped-room");
    let _screen_tool = install_screen_tool(&tools.screen_tool);
    let mut room = TestRoom::new("computer-stopped-room");
    room.enable_browser_controller(&tools).await;
    room.runtime
        .stop_room_environment(&room.session_id)
        .unwrap();
    let before = room
        .runtime
        .room_environment_snapshot(&room.session_id)
        .unwrap();
    let result = room
        .runtime
        .execute_computer_input_as_agent(
            &room.session_id,
            &room.agent_id,
            RoomComputerInputAction::PointerMove { x: 10, y: 10 },
        )
        .await;
    let after = room
        .runtime
        .room_environment_snapshot(&room.session_id)
        .unwrap();
    room.stop_browser_controller().await;
    assert!(result.is_err());
    assert_eq!(after.lifecycle, EnvironmentLifecycle::Stopped);
    assert_eq!(after.runtime_generation, before.runtime_generation);
    assert_eq!(after.tabs, before.tabs);
    assert_eq!(after.health, before.health);
}

#[tokio::test]
async fn foreign_agent_cannot_trigger_computer_input_recovery() {
    let tools = TestTools::new("computer-foreign-agent");
    let _screen_tool = install_screen_tool(&tools.screen_tool);
    let mut room = TestRoom::new("computer-foreign-agent");
    room.enable_browser_controller(&tools).await;
    degrade(&room);
    let foreign = TestRoom::new("computer-foreign-session");
    let foreign_agent = foreign
        .runtime
        .owned
        .agent_store
        .get_agent(&foreign.agent_id)
        .unwrap();
    // Restore a normally created agent into the home store, as durable recovery
    // does, without making it a member of the target Room.
    room.runtime.owned.agent_store.restore_agent(foreign_agent);
    let before = room
        .runtime
        .room_environment_snapshot(&room.session_id)
        .unwrap();
    let result = room
        .runtime
        .execute_computer_input_as_agent(
            &room.session_id,
            &foreign.agent_id,
            RoomComputerInputAction::PointerMove { x: 10, y: 10 },
        )
        .await;
    let after = room
        .runtime
        .room_environment_snapshot(&room.session_id)
        .unwrap();
    room.stop_browser_controller().await;
    assert!(matches!(
        result,
        Err(crate::error::DaemonError::AgentNotInSession { .. })
    ));
    assert_eq!(after.runtime_generation, before.runtime_generation);
    assert_eq!(after.health, before.health);
    assert_eq!(after.tabs, before.tabs);
}

#[tokio::test]
async fn computer_input_recovery_does_not_rewrite_approved_generation() {
    let tools = TestTools::new("computer-stale-approval");
    let _screen_tool = install_screen_tool(&tools.screen_tool);
    let mut room = TestRoom::new("computer-stale-approval");
    room.enable_browser_controller(&tools).await;
    degrade(&room);
    let result = room
        .runtime
        .execute_computer_input_as_agent_for_generation(
            &room.session_id,
            &room.agent_id,
            RoomComputerInputAction::PointerMove { x: 10, y: 10 },
            Some(0),
        )
        .await;
    room.stop_browser_controller().await;
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Room Environment changed after approval"));
}

#[tokio::test]
async fn computer_input_secret_target_recovers_before_approval() {
    let tools = TestTools::new("computer-secret-target");
    let _screen_tool = install_screen_tool(&tools.screen_tool);
    let mut room = TestRoom::new("computer-secret-target");
    room.enable_browser_controller(&tools).await;
    degrade(&room);
    let result = room
        .runtime
        .computer_secret_input_target(&room.session_id, &room.agent_id)
        .await;
    let environment = room
        .runtime
        .room_environment_snapshot(&room.session_id)
        .unwrap();
    room.stop_browser_controller().await;
    // The fixture deliberately has no native target. Recovery must precede the
    // target lookup, while an invalid target still cannot create an approval.
    assert!(result.is_err());
    assert!(environment
        .health
        .iter()
        .filter(|health| matches!(
            health.component,
            EnvironmentComponent::BrowserController | EnvironmentComponent::Browser
        ))
        .all(|health| health.state == EnvironmentComponentHealthState::Ready));
    assert_eq!(environment.tabs[0].title, "After input");
}

#[tokio::test]
async fn human_computer_input_reconciles_degraded_room_with_desktop_authority() {
    let tools = TestTools::new("computer-human-recovery");
    let _screen_tool = install_screen_tool(&tools.screen_tool);
    let mut room = TestRoom::new("computer-human-recovery");
    room.enable_browser_controller(&tools).await;
    let actor = crate::session::EnvironmentActor::new(
        "human:recovery",
        crate::session::EnvironmentActorKind::Human,
        "Operator",
    );
    room.runtime
        .request_room_environment_takeover_as_actor(
            &room.session_id,
            actor.clone(),
            crate::session::InputTarget::Desktop,
        )
        .unwrap();
    degrade(&room);
    let before = room
        .runtime
        .room_environment_snapshot(&room.session_id)
        .unwrap();
    let result = room
        .runtime
        .execute_human_room_environment_action(
            crate::local::SubmitRoomEnvironmentActionRequest {
                session_id: room.session_id.clone(),
                runtime_generation: before.runtime_generation,
                viewport_revision: before.viewport.revision,
                idempotency_key: "human-recovery".into(),
                action: crate::local::RoomEnvironmentHumanAction::PointerMove { x: 10, y: 10 },
            },
            actor,
        )
        .await;
    let after = room
        .runtime
        .room_environment_snapshot(&room.session_id)
        .unwrap();
    room.stop_browser_controller().await;
    assert!(result.is_err()); // No verified headed desktop in this local fixture.
    assert_eq!(after.runtime_generation, before.runtime_generation);
    assert!(after
        .health
        .iter()
        .filter(|health| matches!(
            health.component,
            EnvironmentComponent::BrowserController | EnvironmentComponent::Browser
        ))
        .all(|health| health.state == EnvironmentComponentHealthState::Ready));
    assert_eq!(after.tabs[0].title, "After input");
}
