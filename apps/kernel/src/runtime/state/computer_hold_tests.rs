//! MP-08/MP-10/MP-11: kernel-owned holds, identity, takeover and at-most-once input.
use super::browser_controller_action_execution_runtime_state::computer_input_reconcile_test_support::{install_screen_tool, TestRoom, TestTools};
use crate::error::DaemonError;
use crate::session::{agent_environment_actor_id,EnvironmentActionState,EnvironmentActionArguments,EnvironmentActor,InputTarget};
use crate::local::{RoomEnvironmentHumanAction,SubmitRoomEnvironmentActionRequest};
use crate::transport::room_browser_controller::RoomComputerInputAction;
use std::time::Duration;
fn human_actor() -> EnvironmentActor {
    EnvironmentActor::new(
        "human:hold-test",
        crate::session::EnvironmentActorKind::Human,
        "Operator",
    )
}
fn grant_desktop_to_human(room: &TestRoom, actor: EnvironmentActor) {
    room.runtime
        .request_room_environment_takeover_as_actor(&room.session_id, actor, InputTarget::Desktop)
        .unwrap();
}
#[tokio::test]
async fn mp08_mp10_mp11_computer_hold_keeps_room_tab_authority_and_cancels_on_takeover() {
    crate::test_support::isolated_env_test!();
    let tools = TestTools::new("hold-takeover");
    let marker = tools.screen_tool.with_extension("pressed");
    let reset = tools.screen_tool.with_extension("released");
    std::fs::write(&tools.screen_tool, format!(
            "#!/bin/sh\nset -eu\ncase \"$1\" in\ncomputer-key-hold-stdin) cat >/dev/null; : > '{}'; sleep 30;;\ncomputer-input-reset) : > '{}';;\nesac\n",
            marker.display(),reset.display(),
        )).unwrap();
    let _screen_tool = install_screen_tool(&tools.screen_tool);
    let mut room = TestRoom::new("hold-takeover");
    room.enable_browser_controller(&tools).await;
    let before = room
        .runtime
        .room_environment_snapshot(&room.session_id)
        .unwrap();
    let execution = room.runtime.execute_computer_input_as_agent(
        &room.session_id,
        &room.agent_id,
        RoomComputerInputAction::KeyboardHold {
            input: crate::transport::room_browser_controller::RoomComputerKeyboardInput::new(
                "shift+F8".into(),
            ),
            duration_ms: 10000,
        },
    );
    tokio::pin!(execution);
    tokio::select! {
        result = &mut execution => panic!("hold finished before cancellation: {result:?}"),
        _ = async { while !marker.exists() { tokio::time::sleep(Duration::from_millis(10)).await; } } => {},
        _ = tokio::time::sleep(Duration::from_secs(3)) => panic!("hold was not dispatched"),
    }
    let during = room
        .runtime
        .room_environment_snapshot(&room.session_id)
        .unwrap();
    assert_eq!(during.environment_id, before.environment_id);
    assert_eq!(during.runtime_generation, before.runtime_generation);
    assert_eq!(during.tabs[0].tab_id, before.tabs[0].tab_id);
    let action = during.actions.last().unwrap();
    assert_eq!(action.state, EnvironmentActionState::Running);
    assert_eq!(action.actor_id, agent_environment_actor_id(&room.agent_id));
    assert_eq!(
        action.arguments,
        Some(crate::session::EnvironmentActionArguments::KeyboardHold { duration_ms: 10000 })
    );
    assert!(!serde_json::to_string(action).unwrap().contains("shift+F8"));
    let human = crate::session::EnvironmentActor::new(
        "human:takeover",
        crate::session::EnvironmentActorKind::Human,
        "Operator",
    );
    room.runtime
        .request_room_environment_takeover_as_actor(
            &room.session_id,
            human,
            crate::session::InputTarget::Desktop,
        )
        .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(3), &mut execution)
        .await
        .expect("takeover must settle the owned helper");
    assert!(
        matches!(
            result,
            Err(DaemonError::BrowserControllerActionCancelled { .. })
        ),
        "{result:?}"
    );
    assert!(
        reset.exists(),
        "release/reset must occur before cancelled acknowledgement"
    );
    let after = room
        .runtime
        .room_environment_snapshot(&room.session_id)
        .unwrap();
    assert_eq!(
        after.actions.last().unwrap().state,
        EnvironmentActionState::Cancelled
    );
    assert_eq!(after.tabs[0].tab_id, before.tabs[0].tab_id);
    assert_eq!(after.environment_id, before.environment_id);
    room.stop_browser_controller().await;
}

#[tokio::test]
async fn mp08_mp10_mp11_computer_hold_denies_a_foreign_room_before_helper_dispatch() {
    crate::test_support::isolated_env_test!();
    let room = TestRoom::new("hold-foreign");
    let result = room
        .runtime
        .execute_computer_input_as_agent(
            "foreign-room",
            &room.agent_id,
            RoomComputerInputAction::PointerHold {
                x: 12,
                y: 24,
                button: crate::transport::room_browser_controller::RoomComputerPointerButton::Left,
                duration_ms: 1,
            },
        )
        .await;
    assert!(matches!(result, Err(DaemonError::AgentNotInSession { .. })));
    assert!(room
        .runtime
        .room_environment_snapshot(&room.session_id)
        .unwrap()
        .actions
        .is_empty());
}
#[tokio::test]
async fn mp08_mp10_mp11_human_computer_hold_reuses_takeover_freshness_and_idempotency() {
    crate::test_support::isolated_env_test!();
    let tools = TestTools::new("human-hold");
    let _screen_tool = install_screen_tool(&tools.screen_tool);
    let room = TestRoom::new("human-hold");
    let before = room
        .runtime
        .room_environment_snapshot(&room.session_id)
        .unwrap();
    let actor = human_actor();
    let request = SubmitRoomEnvironmentActionRequest {
        session_id: room.session_id.clone(),
        runtime_generation: before.runtime_generation,
        viewport_revision: before.viewport.revision,
        idempotency_key: "hold-once".into(),
        action: RoomEnvironmentHumanAction::KeyboardHold {
            key: crate::local::RoomEnvironmentKeyboardInput::new("shift+Left".into()),
            duration_ms: 750,
        },
    };
    assert!(room
        .runtime
        .execute_human_room_environment_action(request.clone(), actor.clone())
        .await
        .is_err());
    grant_desktop_to_human(&room, actor.clone());
    let mut stale = request.clone();
    stale.runtime_generation += 1;
    assert!(room
        .runtime
        .execute_human_room_environment_action(stale, actor.clone())
        .await
        .is_err());
    let (id, after) = room
        .runtime
        .execute_human_room_environment_action(request.clone(), actor.clone())
        .await
        .unwrap();
    assert_eq!(after.tabs[0].tab_id, before.tabs[0].tab_id);
    assert_eq!(after.environment_id, before.environment_id);
    assert_eq!(
        after.actions.last().unwrap().arguments,
        Some(EnvironmentActionArguments::KeyboardHold { duration_ms: 750 })
    );
    assert!(!serde_json::to_string(after.actions.last().unwrap())
        .unwrap()
        .contains("shift+Left"));
    let (repeat_id, repeat) = room
        .runtime
        .execute_human_room_environment_action(request.clone(), actor.clone())
        .await
        .unwrap();
    assert_eq!(repeat_id, id);
    assert_eq!(repeat.actions.len(), after.actions.len());
    let mut changed = request.clone();
    changed.action = RoomEnvironmentHumanAction::KeyboardHold {
        key: crate::local::RoomEnvironmentKeyboardInput::new("shift+Right".into()),
        duration_ms: 750,
    };
    assert!(room
        .runtime
        .execute_human_room_environment_action(changed, actor.clone())
        .await
        .is_err());
    let mut changed_duration = request;
    changed_duration.action = RoomEnvironmentHumanAction::KeyboardHold {
        key: crate::local::RoomEnvironmentKeyboardInput::new("shift+Left".into()),
        duration_ms: 751,
    };
    assert!(room
        .runtime
        .execute_human_room_environment_action(changed_duration, actor)
        .await
        .is_err());
}
#[tokio::test]
async fn mp08_mp10_mp11_computer_hold_timeout_releases_before_failed_action() {
    crate::test_support::isolated_env_test!();
    let tools = TestTools::new("hold-timeout");
    let reset = tools.screen_tool.with_extension("released");
    std::fs::write(&tools.screen_tool,format!(
        "#!/bin/sh\nset -eu\ncase \"$1\" in\ncomputer-key-hold-stdin) cat >/dev/null; sleep 30;;\ncomputer-input-reset) : > '{}';;\nesac\n",reset.display()
    )).unwrap();
    let _screen_tool = install_screen_tool(&tools.screen_tool);
    let room = TestRoom::new("hold-timeout");
    let result = tokio::time::timeout(
        Duration::from_secs(8),
        room.runtime.execute_computer_input_as_agent(
            &room.session_id,
            &room.agent_id,
            RoomComputerInputAction::KeyboardHold {
                input: crate::transport::room_browser_controller::RoomComputerKeyboardInput::new(
                    "shift+F8".into(),
                ),
                duration_ms: 1,
            },
        ),
    )
    .await
    .expect("a hung native helper must settle within the shared hold deadline");
    assert!(result.is_err());
    assert!(
        reset.exists(),
        "timed-out physical input must reset before terminal acknowledgement"
    );
    assert_eq!(
        room.runtime
            .room_environment_snapshot(&room.session_id)
            .unwrap()
            .actions
            .last()
            .unwrap()
            .state,
        EnvironmentActionState::Failed
    );
}

#[cfg(unix)]
#[tokio::test]
async fn mp08_mp10_mp11_computer_hold_abnormal_child_exit_releases_before_failed_action() {
    crate::test_support::isolated_env_test!();
    let tools = TestTools::new("hold-abnormal-exit");
    let held = tools.screen_tool.with_extension("pressed");
    let reset = tools.screen_tool.with_extension("released");
    let child = tools.screen_tool.with_extension("py");
    // Bash returns 128+signal for a child death, just like production slice-screen.sh.
    // No core files, and the synthetic child signals only its own validated PID.
    std::fs::write(&child, format!(
        "import os,signal,sys\nfrom pathlib import Path\nPath({:?}).touch()\npid=os.getpid()\nassert pid > 1\nos.kill(pid,int(sys.argv[1]))\n",
        held.to_str().unwrap(),
    )).unwrap();
    let _screen_tool = install_screen_tool(&tools.screen_tool);
    let room = TestRoom::new("hold-abnormal-exit");
    for signum in [libc::SIGQUIT, libc::SIGABRT, libc::SIGSEGV, libc::SIGKILL] {
        std::fs::write(&tools.screen_tool, format!(
            "#!/bin/bash\nset -u\nulimit -c 0\ncase \"$1\" in\ncomputer-key-hold-stdin) cat >/dev/null; python3 '{}' {}; exit $?;;\ncomputer-input-reset) rm -f '{}'; : > '{}';;\nesac\n",
            child.display(), signum, held.display(), reset.display(),
        )).unwrap();
        let result = room
            .runtime
            .execute_computer_input_as_agent(
                &room.session_id,
                &room.agent_id,
                RoomComputerInputAction::KeyboardHold {
                    input:
                        crate::transport::room_browser_controller::RoomComputerKeyboardInput::new(
                            "shift+F8".into(),
                        ),
                    duration_ms: 1000,
                },
            )
            .await;
        assert!(result.is_err(), "signal {signum} must fail the Action");
        let after = room
            .runtime
            .room_environment_snapshot(&room.session_id)
            .unwrap();
        assert_eq!(
            after.actions.last().unwrap().state,
            EnvironmentActionState::Failed
        );
        assert!(
            reset.exists() && !held.exists(),
            "signal {signum}: release must precede failed acknowledgement"
        );
        std::fs::remove_file(&reset).unwrap();
    }
}

#[tokio::test]
async fn mp08_mp10_mp11_computer_hold_prepress_denial_preserves_foreign_input() {
    crate::test_support::isolated_env_test!();
    let tools = TestTools::new("hold-prepress-denial");
    let held = tools.screen_tool.with_extension("foreign-pressed");
    let reset = tools.screen_tool.with_extension("released");
    std::fs::write(&held, "foreign actor").unwrap();
    std::fs::write(&tools.screen_tool, format!(
        "#!/bin/sh\ncase \"$1\" in\ncomputer-key-hold-stdin) cat >/dev/null; exit 1;;\ncomputer-input-reset) rm -f '{}'; : > '{}';;\nesac\n",
        held.display(), reset.display(),
    )).unwrap();
    let _screen_tool = install_screen_tool(&tools.screen_tool);
    let room = TestRoom::new("hold-prepress-denial");
    let result = room
        .runtime
        .execute_computer_input_as_agent(
            &room.session_id,
            &room.agent_id,
            RoomComputerInputAction::KeyboardHold {
                input: crate::transport::room_browser_controller::RoomComputerKeyboardInput::new(
                    "F8".into(),
                ),
                duration_ms: 1000,
            },
        )
        .await;
    assert!(result.is_err());
    assert!(
        held.exists() && !reset.exists(),
        "pre-press denial must not clear another actor's input"
    );
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "MP-08/MP-10/MP-11: requires owned X11 fixture and exact-source --test-binary drill"]
async fn mp08_mp10_mp11_physical_child_death_resets_before_failed_acknowledgement() {
    crate::test_support::isolated_env_test!();
    let tool = std::path::PathBuf::from(
        std::env::var("CHARIOX_B204_PHYSICAL_SCREEN_TOOL")
            .expect("physical screen wrapper required"),
    );
    assert!(tool.is_absolute() && tool.is_file());
    let _screen_tool = install_screen_tool(&tool);
    let room = TestRoom::new("physical-hold-child-exit");
    let actions = [
        RoomComputerInputAction::KeyboardHold {
            input: crate::transport::room_browser_controller::RoomComputerKeyboardInput::new(
                "shift+F8".into(),
            ),
            duration_ms: 10000,
        },
        RoomComputerInputAction::PointerHold {
            x: 100,
            y: 280,
            button: crate::transport::room_browser_controller::RoomComputerPointerButton::Left,
            duration_ms: 10000,
        },
    ];
    for action in actions {
        let kind = match &action {
            RoomComputerInputAction::KeyboardHold { .. } => "computer-key-hold-stdin",
            _ => "pointer-hold",
        };
        let result = room
            .runtime
            .execute_computer_input_as_agent(&room.session_id, &room.agent_id, action)
            .await;
        assert!(
            result.is_err(),
            "fatal physical helper child must fail the Action"
        );
        assert_eq!(
            room.runtime
                .room_environment_snapshot(&room.session_id)
                .unwrap()
                .actions
                .last()
                .unwrap()
                .state,
            EnvironmentActionState::Failed
        );
        // Query actual X11 state after the failed acknowledgement. Reset must
        // already have released every key/button, not merely produced an artifact.
        let status = std::process::Command::new(&tool)
            .arg("assert-released")
            .arg(kind)
            .status()
            .unwrap();
        assert!(
            status.success(),
            "physical input remained pressed at failure acknowledgement"
        );
    }
}
