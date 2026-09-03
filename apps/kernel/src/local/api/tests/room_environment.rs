use super::*;
use crate::session::CanonicalViewport;

#[test]
fn room_environment_takeover_and_release_use_authenticated_actor_and_room_lane() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(
                "workspace-environment-takeover",
                "worktree-environment-takeover",
            ),
        ))
        .expect("Room should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected local response: {other:?}"),
    };
    harness
        .dispatch(LocalDaemonRequest::StartRoomEnvironment(
            StartRoomEnvironmentRequest {
                session_id: session.id().to_string(),
                viewport: RoomEnvironmentViewportRequest {
                    css_width: 1280,
                    css_height: 800,
                    device_scale_factor: 1,
                    desktop_pixel_width: 1280,
                    desktop_pixel_height: 800,
                },
            },
        ))
        .expect("Room Environment should start");
    harness.with_app_mut(|app| {
        app.session_state_store()
            .transition_room_environment(session.id(), crate::session::EnvironmentLifecycle::Ready)
            .expect("managed runtime should become ready");
    });

    let response = harness
        .dispatch(LocalDaemonRequest::RequestRoomEnvironmentInputTakeover(
            RequestRoomEnvironmentInputTakeoverRequest {
                session_id: session.id().to_string(),
                target: crate::session::InputTarget::Desktop,
            },
        ))
        .expect("authenticated Room member should take desktop input");
    let LocalDaemonResponse::RoomEnvironmentTakeoverUpdated {
        outcome,
        environment,
    } = response
    else {
        panic!("unexpected local response: {response:?}");
    };
    assert_eq!(outcome, crate::session::TakeoverOutcome::Granted);
    assert_eq!(environment.input_ownership.len(), 1);
    assert_eq!(
        environment.input_ownership[0].actor_id,
        crate::session::human_environment_actor_id(crate::session::DEFAULT_LOCAL_USER_ID)
    );
    assert_eq!(
        environment.input_ownership[0].target,
        crate::session::InputTarget::Desktop
    );

    let response = harness
        .dispatch(LocalDaemonRequest::ReleaseRoomEnvironmentInput(
            ReleaseRoomEnvironmentInputRequest {
                session_id: session.id().to_string(),
                target: crate::session::InputTarget::Desktop,
            },
        ))
        .expect("authenticated Room member should release desktop input");
    let LocalDaemonResponse::RoomEnvironmentInputReleased { environment } = response else {
        panic!("unexpected local response: {response:?}");
    };
    assert!(environment.input_ownership.is_empty());
}

#[test]
fn room_environment_action_cancel_uses_authenticated_actor_and_stable_errors() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch_as_user(
            "owner-1",
            LocalDaemonRequest::CreateSession(CreateSessionRequest::new(
                "workspace-environment-cancel",
                "worktree-environment-cancel",
            )),
        )
        .expect("Room should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected local response: {other:?}"),
    };
    harness
        .dispatch_as_user(
            "owner-1",
            LocalDaemonRequest::StartRoomEnvironment(StartRoomEnvironmentRequest {
                session_id: session.id().to_string(),
                viewport: RoomEnvironmentViewportRequest {
                    css_width: 1280,
                    css_height: 800,
                    device_scale_factor: 1,
                    desktop_pixel_width: 1280,
                    desktop_pixel_height: 800,
                },
            }),
        )
        .expect("Room Environment should start");
    harness.with_app_mut(|app| {
        app.session_state_store()
            .transition_room_environment(session.id(), crate::session::EnvironmentLifecycle::Ready)
            .expect("Room Environment should become ready");
    });

    let error = harness
        .dispatch_as_user(
            "owner-1",
            LocalDaemonRequest::CancelRoomEnvironmentAction(CancelRoomEnvironmentActionRequest {
                session_id: session.id().to_string(),
                action_id: "action-missing".to_string(),
            }),
        )
        .expect_err("an unknown Action should not be cancelled");
    assert!(matches!(
        error,
        DaemonError::LocalTransport {
            operation: "environment.action.cancel",
            message,
        } if message.starts_with("environment_unknown_action:")
    ));
}

#[test]
fn room_environment_human_input_requires_takeover_and_rejects_invalid_arguments() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch_as_user(
            "owner-1",
            LocalDaemonRequest::CreateSession(CreateSessionRequest::new(
                "workspace-environment-pointer-click",
                "worktree-environment-pointer-click",
            )),
        )
        .expect("Room should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected local response: {other:?}"),
    };
    harness
        .dispatch_as_user(
            "owner-1",
            LocalDaemonRequest::StartRoomEnvironment(StartRoomEnvironmentRequest {
                session_id: session.id().to_string(),
                viewport: RoomEnvironmentViewportRequest {
                    css_width: 1280,
                    css_height: 800,
                    device_scale_factor: 1,
                    desktop_pixel_width: 1280,
                    desktop_pixel_height: 800,
                },
            }),
        )
        .expect("Room Environment should start");
    harness.with_app_mut(|app| {
        app.session_state_store()
            .transition_room_environment(session.id(), crate::session::EnvironmentLifecycle::Ready)
            .expect("Room Environment should become ready");
    });
    let environment = harness.with_app(|app| {
        app.session_state_store()
            .room_environment_snapshot(session.id())
            .expect("Room Environment should exist")
    });
    let request = |runtime_generation: u64,
                   viewport_revision: u64,
                   idempotency_key: String,
                   x: u32,
                   y: u32,
                   click_count: u8| {
        LocalDaemonRequest::SubmitRoomEnvironmentAction(SubmitRoomEnvironmentActionRequest {
            session_id: session.id().to_string(),
            runtime_generation,
            viewport_revision,
            idempotency_key,
            action: RoomEnvironmentHumanAction::PointerClick {
                x,
                y,
                button: RoomEnvironmentPointerButton::Left,
                click_count,
            },
        })
    };

    let error = harness
        .dispatch_as_user(
            "owner-1",
            request(
                environment.runtime_generation,
                environment.viewport.revision,
                "click-no-takeover".to_string(),
                20,
                30,
                1,
            ),
        )
        .expect_err("human input must require explicit desktop takeover");
    assert!(matches!(
        error,
        DaemonError::LocalTransport {
            operation: "environment.action.submit",
            message,
        } if message.starts_with("environment_input_takeover_required:")
    ));

    harness
        .dispatch_as_user(
            "owner-1",
            LocalDaemonRequest::RequestRoomEnvironmentInputTakeover(
                RequestRoomEnvironmentInputTakeoverRequest {
                    session_id: session.id().to_string(),
                    target: crate::session::InputTarget::Desktop,
                },
            ),
        )
        .expect("owner should take desktop input");

    let human_request = |idempotency_key: &str, action: RoomEnvironmentHumanAction| {
        LocalDaemonRequest::SubmitRoomEnvironmentAction(SubmitRoomEnvironmentActionRequest {
            session_id: session.id().to_string(),
            runtime_generation: environment.runtime_generation,
            viewport_revision: environment.viewport.revision,
            idempotency_key: idempotency_key.to_string(),
            action,
        })
    };

    for (candidate, expected_code) in [
        (
            request(
                environment.runtime_generation + 1,
                environment.viewport.revision,
                "click-stale-runtime".to_string(),
                20,
                30,
                1,
            ),
            "environment_stale_runtime_generation:",
        ),
        (
            request(
                environment.runtime_generation,
                environment.viewport.revision + 1,
                "click-stale-viewport".to_string(),
                20,
                30,
                1,
            ),
            "environment_stale_viewport_revision:",
        ),
        (
            request(
                environment.runtime_generation,
                environment.viewport.revision,
                "click-out-of-bounds".to_string(),
                environment.viewport.desktop_pixel_width,
                30,
                1,
            ),
            "environment_pointer_out_of_bounds:",
        ),
        (
            request(
                environment.runtime_generation,
                environment.viewport.revision,
                "click-count".to_string(),
                20,
                30,
                0,
            ),
            "environment_invalid_click_count:",
        ),
        (
            request(
                environment.runtime_generation,
                environment.viewport.revision,
                "   ".to_string(),
                20,
                30,
                1,
            ),
            "environment_invalid_idempotency_key:",
        ),
        (
            request(
                environment.runtime_generation,
                environment.viewport.revision,
                "a".repeat(129),
                20,
                30,
                1,
            ),
            "environment_invalid_idempotency_key:",
        ),
        (
            human_request(
                "move-out-of-bounds",
                RoomEnvironmentHumanAction::PointerMove {
                    x: environment.viewport.desktop_pixel_width,
                    y: 30,
                },
            ),
            "environment_pointer_out_of_bounds:",
        ),
        (
            human_request(
                "drag-start-out-of-bounds",
                RoomEnvironmentHumanAction::PointerDrag {
                    from_x: environment.viewport.desktop_pixel_width,
                    from_y: 30,
                    to_x: 40,
                    to_y: 50,
                    button: RoomEnvironmentPointerButton::Left,
                },
            ),
            "environment_pointer_out_of_bounds:",
        ),
        (
            human_request(
                "drag-end-out-of-bounds",
                RoomEnvironmentHumanAction::PointerDrag {
                    from_x: 20,
                    from_y: 30,
                    to_x: 40,
                    to_y: environment.viewport.desktop_pixel_height,
                    button: RoomEnvironmentPointerButton::Right,
                },
            ),
            "environment_pointer_out_of_bounds:",
        ),
        (
            human_request(
                "scroll-zero",
                RoomEnvironmentHumanAction::PointerScroll {
                    x: 20,
                    y: 30,
                    horizontal_steps: 0,
                    vertical_steps: 0,
                },
            ),
            "environment_invalid_scroll_steps:",
        ),
        (
            human_request(
                "scroll-too-large",
                RoomEnvironmentHumanAction::PointerScroll {
                    x: 20,
                    y: 30,
                    horizontal_steps: 121,
                    vertical_steps: 0,
                },
            ),
            "environment_invalid_scroll_steps:",
        ),
        (
            human_request(
                "keyboard-text-empty",
                RoomEnvironmentHumanAction::KeyboardText {
                    text: RoomEnvironmentKeyboardInput::new(String::new()),
                },
            ),
            "environment_invalid_keyboard_text:",
        ),
        (
            human_request(
                "keyboard-text-too-large",
                RoomEnvironmentHumanAction::KeyboardText {
                    text: RoomEnvironmentKeyboardInput::new("a".repeat(64 * 1024 + 1)),
                },
            ),
            "environment_invalid_keyboard_text:",
        ),
        (
            human_request(
                "keyboard-key-invalid",
                RoomEnvironmentHumanAction::KeyboardKey {
                    key: RoomEnvironmentKeyboardInput::new("ctrl shift p".to_string()),
                    repeat: 1,
                },
            ),
            "environment_invalid_keyboard_key:",
        ),
        (
            human_request(
                "keyboard-repeat-zero",
                RoomEnvironmentHumanAction::KeyboardKey {
                    key: RoomEnvironmentKeyboardInput::new("Return".to_string()),
                    repeat: 0,
                },
            ),
            "environment_invalid_keyboard_repeat:",
        ),
        (
            human_request(
                "keyboard-repeat-too-large",
                RoomEnvironmentHumanAction::KeyboardKey {
                    key: RoomEnvironmentKeyboardInput::new("Return".to_string()),
                    repeat: 33,
                },
            ),
            "environment_invalid_keyboard_repeat:",
        ),
    ] {
        let error = harness
            .dispatch_as_user("owner-1", candidate)
            .expect_err("invalid pointer input must fail before worker execution");
        assert!(matches!(
            error,
            DaemonError::LocalTransport {
                operation: "environment.action.submit",
                message,
            } if message.starts_with(expected_code)
        ));
    }
}

#[test]
fn room_environment_pointer_click_executes_once_and_returns_terminal_state() {
    let _guard = crate::env_lock::lock();
    let root = std::env::temp_dir().join(format!(
        "chariox-human-pointer-click-test-{}",
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&root).expect("test root should be created");
    let script = root.join("slice-screen.sh");
    let log = root.join("pointer-click.log");
    std::fs::write(
        &script,
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$CHARIOX_POINTER_CLICK_LOG\"\n",
    )
    .expect("screen helper should be written");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))
            .expect("screen helper should be executable");
    }
    std::env::set_var("CHARIOX_SLICE_SCREEN_TOOL", &script);
    std::env::set_var("CHARIOX_POINTER_CLICK_LOG", &log);
    std::env::set_var("CHARIOX_SLICE_SCREEN_TOOL_TIMEOUT_MS", "5000");

    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch_as_user(
            "owner-1",
            LocalDaemonRequest::CreateSession(CreateSessionRequest::new(
                "workspace-environment-pointer-click-execution",
                "worktree-environment-pointer-click-execution",
            )),
        )
        .expect("Room should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected local response: {other:?}"),
    };
    harness
        .dispatch_as_user(
            "owner-1",
            LocalDaemonRequest::StartRoomEnvironment(StartRoomEnvironmentRequest {
                session_id: session.id().to_string(),
                viewport: RoomEnvironmentViewportRequest {
                    css_width: 1280,
                    css_height: 800,
                    device_scale_factor: 1,
                    desktop_pixel_width: 1280,
                    desktop_pixel_height: 800,
                },
            }),
        )
        .expect("Room Environment should start");
    harness.with_app_mut(|app| {
        app.session_state_store()
            .transition_room_environment(session.id(), crate::session::EnvironmentLifecycle::Ready)
            .expect("Room Environment should become ready");
    });
    harness
        .dispatch_as_user(
            "owner-1",
            LocalDaemonRequest::RequestRoomEnvironmentInputTakeover(
                RequestRoomEnvironmentInputTakeoverRequest {
                    session_id: session.id().to_string(),
                    target: crate::session::InputTarget::Desktop,
                },
            ),
        )
        .expect("owner should take desktop input");
    let environment = harness.with_app(|app| {
        app.session_state_store()
            .room_environment_snapshot(session.id())
            .expect("Room Environment should exist")
    });
    let request = || {
        LocalDaemonRequest::SubmitRoomEnvironmentAction(SubmitRoomEnvironmentActionRequest {
            session_id: session.id().to_string(),
            runtime_generation: environment.runtime_generation,
            viewport_revision: environment.viewport.revision,
            idempotency_key: "pointer-click-1".to_string(),
            action: RoomEnvironmentHumanAction::PointerClick {
                x: 320,
                y: 180,
                button: RoomEnvironmentPointerButton::Right,
                click_count: 2,
            },
        })
    };

    let first = harness
        .dispatch_as_user("owner-1", request())
        .expect("valid human pointer click should execute");
    let LocalDaemonResponse::RoomEnvironmentActionSubmitted {
        action_id,
        environment,
    } = first
    else {
        panic!("unexpected local response: {first:?}");
    };
    assert_eq!(
        environment
            .actions
            .iter()
            .find(|action| action.action_id == action_id)
            .map(|action| action.state),
        Some(crate::session::EnvironmentActionState::Completed)
    );
    assert_eq!(
        environment
            .actions
            .iter()
            .find(|action| action.action_id == action_id)
            .and_then(|action| action.arguments.clone()),
        Some(crate::session::EnvironmentActionArguments::PointerClick {
            x: 320,
            y: 180,
            button: crate::session::EnvironmentPointerButton::Right,
            click_count: 2,
            viewport_revision: environment.viewport.revision,
        })
    );

    harness
        .dispatch_as_user(
            "owner-1",
            LocalDaemonRequest::UpdateRoomEnvironmentViewport(
                UpdateRoomEnvironmentViewportRequest {
                    session_id: session.id().to_string(),
                    expected_revision: environment.viewport.revision,
                    viewport: RoomEnvironmentViewportRequest {
                        css_width: 1024,
                        css_height: 768,
                        device_scale_factor: 1,
                        desktop_pixel_width: 1024,
                        desktop_pixel_height: 768,
                    },
                },
            ),
        )
        .expect("the viewport should be allowed to change after the click");

    harness
        .dispatch_as_user(
            "owner-1",
            LocalDaemonRequest::ReleaseRoomEnvironmentInput(ReleaseRoomEnvironmentInputRequest {
                session_id: session.id().to_string(),
                target: crate::session::InputTarget::Desktop,
            }),
        )
        .expect("the actor should be allowed to release desktop input after the click");

    let retry = harness
        .dispatch_as_user("owner-1", request())
        .expect("an idempotent retry should return the original Action after input release");
    let LocalDaemonResponse::RoomEnvironmentActionSubmitted {
        action_id: retry_action_id,
        environment: retry_environment,
    } = retry
    else {
        panic!("unexpected local response: {retry:?}");
    };
    assert_eq!(retry_action_id, action_id);
    assert_eq!(retry_environment.viewport.revision, 2);
    assert_eq!(
        retry_environment
            .actions
            .iter()
            .find(|action| action.action_id == action_id)
            .map(|action| action.state),
        Some(crate::session::EnvironmentActionState::Completed)
    );
    let conflict = harness
        .dispatch_as_user(
            "owner-1",
            LocalDaemonRequest::SubmitRoomEnvironmentAction(SubmitRoomEnvironmentActionRequest {
                session_id: session.id().to_string(),
                runtime_generation: environment.runtime_generation,
                viewport_revision: environment.viewport.revision,
                idempotency_key: "pointer-click-1".to_string(),
                action: RoomEnvironmentHumanAction::PointerClick {
                    x: 321,
                    y: 180,
                    button: RoomEnvironmentPointerButton::Right,
                    click_count: 2,
                },
            }),
        )
        .expect_err("reusing an idempotency key for another click must fail");
    assert!(matches!(
        conflict,
        DaemonError::LocalTransport {
            operation: "environment.action.submit",
            message,
        } if message.starts_with("environment_idempotency_conflict:")
    ));
    assert_eq!(
        std::fs::read_to_string(&log).expect("physical click should be logged"),
        "pointer-click 320 180 right 2\n"
    );

    std::env::remove_var("CHARIOX_SLICE_SCREEN_TOOL");
    std::env::remove_var("CHARIOX_POINTER_CLICK_LOG");
    std::env::remove_var("CHARIOX_SLICE_SCREEN_TOOL_TIMEOUT_MS");
    std::fs::remove_dir_all(&root).expect("test root should be removed");
}

#[test]
fn room_environment_pointer_motion_executes_and_records_coordinates() {
    let _guard = crate::env_lock::lock();
    let root = std::env::temp_dir().join(format!(
        "chariox-human-pointer-move-test-{}",
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&root).expect("test root should be created");
    let script = root.join("slice-screen.sh");
    let log = root.join("pointer-move.log");
    std::fs::write(
        &script,
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$CHARIOX_POINTER_MOVE_LOG\"\n",
    )
    .expect("screen helper should be written");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))
            .expect("screen helper should be executable");
    }
    std::env::set_var("CHARIOX_SLICE_SCREEN_TOOL", &script);
    std::env::set_var("CHARIOX_POINTER_MOVE_LOG", &log);
    std::env::set_var("CHARIOX_SLICE_SCREEN_TOOL_TIMEOUT_MS", "5000");

    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch_as_user(
            "owner-1",
            LocalDaemonRequest::CreateSession(CreateSessionRequest::new(
                "workspace-environment-pointer-move-execution",
                "worktree-environment-pointer-move-execution",
            )),
        )
        .expect("Room should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected local response: {other:?}"),
    };
    harness
        .dispatch_as_user(
            "owner-1",
            LocalDaemonRequest::StartRoomEnvironment(StartRoomEnvironmentRequest {
                session_id: session.id().to_string(),
                viewport: RoomEnvironmentViewportRequest {
                    css_width: 1280,
                    css_height: 800,
                    device_scale_factor: 1,
                    desktop_pixel_width: 1280,
                    desktop_pixel_height: 800,
                },
            }),
        )
        .expect("Room Environment should start");
    harness.with_app_mut(|app| {
        app.session_state_store()
            .transition_room_environment(session.id(), crate::session::EnvironmentLifecycle::Ready)
            .expect("Room Environment should become ready");
    });
    harness
        .dispatch_as_user(
            "owner-1",
            LocalDaemonRequest::RequestRoomEnvironmentInputTakeover(
                RequestRoomEnvironmentInputTakeoverRequest {
                    session_id: session.id().to_string(),
                    target: crate::session::InputTarget::Desktop,
                },
            ),
        )
        .expect("owner should take desktop input");
    let environment = harness.with_app(|app| {
        app.session_state_store()
            .room_environment_snapshot(session.id())
            .expect("Room Environment should exist")
    });

    let response = harness
        .dispatch_as_user(
            "owner-1",
            LocalDaemonRequest::SubmitRoomEnvironmentAction(SubmitRoomEnvironmentActionRequest {
                session_id: session.id().to_string(),
                runtime_generation: environment.runtime_generation,
                viewport_revision: environment.viewport.revision,
                idempotency_key: "pointer-move-1".to_string(),
                action: RoomEnvironmentHumanAction::PointerMove { x: 640, y: 400 },
            }),
        )
        .expect("valid human pointer move should execute");
    let LocalDaemonResponse::RoomEnvironmentActionSubmitted {
        action_id,
        environment,
    } = response
    else {
        panic!("unexpected local response: {response:?}");
    };
    let action = environment
        .actions
        .iter()
        .find(|action| action.action_id == action_id)
        .expect("pointer move Action should be recorded");
    assert_eq!(action.kind, "pointer_move");
    assert_eq!(
        action.state,
        crate::session::EnvironmentActionState::Completed
    );
    assert_eq!(
        action.arguments,
        Some(crate::session::EnvironmentActionArguments::PointerMove {
            x: 640,
            y: 400,
            viewport_revision: environment.viewport.revision,
        })
    );
    assert_eq!(
        std::fs::read_to_string(&log).expect("physical move should be logged"),
        "move 640 400\n"
    );

    let response = harness
        .dispatch_as_user(
            "owner-1",
            LocalDaemonRequest::SubmitRoomEnvironmentAction(SubmitRoomEnvironmentActionRequest {
                session_id: session.id().to_string(),
                runtime_generation: environment.runtime_generation,
                viewport_revision: environment.viewport.revision,
                idempotency_key: "pointer-drag-1".to_string(),
                action: RoomEnvironmentHumanAction::PointerDrag {
                    from_x: 120,
                    from_y: 160,
                    to_x: 720,
                    to_y: 560,
                    button: RoomEnvironmentPointerButton::Left,
                },
            }),
        )
        .expect("valid human pointer drag should execute");
    let LocalDaemonResponse::RoomEnvironmentActionSubmitted {
        action_id,
        environment,
    } = response
    else {
        panic!("unexpected local response: {response:?}");
    };
    let action = environment
        .actions
        .iter()
        .find(|action| action.action_id == action_id)
        .expect("pointer drag Action should be recorded");
    assert_eq!(action.kind, "pointer_drag");
    assert_eq!(
        action.state,
        crate::session::EnvironmentActionState::Completed
    );
    assert_eq!(
        action.arguments,
        Some(crate::session::EnvironmentActionArguments::PointerDrag {
            from_x: 120,
            from_y: 160,
            to_x: 720,
            to_y: 560,
            button: crate::session::EnvironmentPointerButton::Left,
            viewport_revision: environment.viewport.revision,
        })
    );
    assert_eq!(
        std::fs::read_to_string(&log).expect("physical motion should be logged"),
        "move 640 400\npointer-drag 120 160 720 560 left\n"
    );

    let response = harness
        .dispatch_as_user(
            "owner-1",
            LocalDaemonRequest::SubmitRoomEnvironmentAction(SubmitRoomEnvironmentActionRequest {
                session_id: session.id().to_string(),
                runtime_generation: environment.runtime_generation,
                viewport_revision: environment.viewport.revision,
                idempotency_key: "pointer-scroll-1".to_string(),
                action: RoomEnvironmentHumanAction::PointerScroll {
                    x: 640,
                    y: 400,
                    horizontal_steps: -3,
                    vertical_steps: 5,
                },
            }),
        )
        .expect("valid human pointer scroll should execute");
    let LocalDaemonResponse::RoomEnvironmentActionSubmitted {
        action_id,
        environment,
    } = response
    else {
        panic!("unexpected local response: {response:?}");
    };
    let action = environment
        .actions
        .iter()
        .find(|action| action.action_id == action_id)
        .expect("pointer scroll Action should be recorded");
    assert_eq!(action.kind, "pointer_scroll");
    assert_eq!(
        action.state,
        crate::session::EnvironmentActionState::Completed
    );
    assert_eq!(
        action.arguments,
        Some(crate::session::EnvironmentActionArguments::PointerScroll {
            x: 640,
            y: 400,
            horizontal_steps: -3,
            vertical_steps: 5,
            viewport_revision: environment.viewport.revision,
        })
    );
    assert_eq!(
        std::fs::read_to_string(&log).expect("physical mouse input should be logged"),
        "move 640 400\npointer-drag 120 160 720 560 left\npointer-scroll 640 400 -3 5\n"
    );

    std::env::remove_var("CHARIOX_SLICE_SCREEN_TOOL");
    std::env::remove_var("CHARIOX_POINTER_MOVE_LOG");
    std::env::remove_var("CHARIOX_SLICE_SCREEN_TOOL_TIMEOUT_MS");
    std::fs::remove_dir_all(&root).expect("test root should be removed");
}

#[test]
fn room_environment_keyboard_input_executes_without_persisting_input() {
    let _guard = crate::env_lock::lock();
    let root = std::env::temp_dir().join(format!(
        "chariox-human-keyboard-text-test-{}",
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&root).expect("test root should be created");
    let script = root.join("slice-screen.sh");
    let command_log = root.join("keyboard-command.log");
    let input_log = root.join("keyboard-input.log");
    std::fs::write(
        &script,
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$CHARIOX_KEYBOARD_COMMAND_LOG\"\ncat >> \"$CHARIOX_KEYBOARD_INPUT_LOG\"\n",
    )
    .expect("screen helper should be written");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))
            .expect("screen helper should be executable");
    }
    std::env::set_var("CHARIOX_SLICE_SCREEN_TOOL", &script);
    std::env::set_var("CHARIOX_KEYBOARD_COMMAND_LOG", &command_log);
    std::env::set_var("CHARIOX_KEYBOARD_INPUT_LOG", &input_log);
    std::env::set_var("CHARIOX_SLICE_SCREEN_TOOL_TIMEOUT_MS", "5000");

    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch_as_user(
            "owner-1",
            LocalDaemonRequest::CreateSession(CreateSessionRequest::new(
                "workspace-environment-keyboard-text",
                "worktree-environment-keyboard-text",
            )),
        )
        .expect("Room should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected local response: {other:?}"),
    };
    harness
        .dispatch_as_user(
            "owner-1",
            LocalDaemonRequest::StartRoomEnvironment(StartRoomEnvironmentRequest {
                session_id: session.id().to_string(),
                viewport: RoomEnvironmentViewportRequest {
                    css_width: 1280,
                    css_height: 800,
                    device_scale_factor: 1,
                    desktop_pixel_width: 1280,
                    desktop_pixel_height: 800,
                },
            }),
        )
        .expect("Room Environment should start");
    harness.with_app_mut(|app| {
        app.session_state_store()
            .transition_room_environment(session.id(), crate::session::EnvironmentLifecycle::Ready)
            .expect("Room Environment should become ready");
    });
    harness
        .dispatch_as_user(
            "owner-1",
            LocalDaemonRequest::RequestRoomEnvironmentInputTakeover(
                RequestRoomEnvironmentInputTakeoverRequest {
                    session_id: session.id().to_string(),
                    target: crate::session::InputTarget::Desktop,
                },
            ),
        )
        .expect("owner should take desktop input");
    let environment = harness.with_app(|app| {
        app.session_state_store()
            .room_environment_snapshot(session.id())
            .expect("Room Environment should exist")
    });
    let input = "Grüße 世界";
    let keyboard_input = RoomEnvironmentKeyboardInput::new(input.to_string());
    assert!(!format!("{keyboard_input:?}").contains(input));

    let response = harness
        .dispatch_as_user(
            "owner-1",
            LocalDaemonRequest::SubmitRoomEnvironmentAction(SubmitRoomEnvironmentActionRequest {
                session_id: session.id().to_string(),
                runtime_generation: environment.runtime_generation,
                viewport_revision: environment.viewport.revision,
                idempotency_key: "keyboard-text-1".to_string(),
                action: RoomEnvironmentHumanAction::KeyboardText {
                    text: keyboard_input,
                },
            }),
        )
        .expect("valid human keyboard text should execute");
    let LocalDaemonResponse::RoomEnvironmentActionSubmitted {
        action_id,
        environment,
    } = response
    else {
        panic!("unexpected local response: {response:?}");
    };
    let action = environment
        .actions
        .iter()
        .find(|action| action.action_id == action_id)
        .expect("keyboard text Action should be recorded");
    assert_eq!(action.kind, "keyboard_text");
    assert_eq!(
        action.state,
        crate::session::EnvironmentActionState::Completed
    );
    assert_eq!(
        action.arguments,
        Some(crate::session::EnvironmentActionArguments::KeyboardText {
            utf8_byte_count: 14,
            character_count: 8,
        })
    );
    assert_eq!(
        std::fs::read_to_string(&command_log).expect("keyboard command should be logged"),
        "computer-type-stdin\n"
    );
    assert_eq!(
        std::fs::read_to_string(&input_log).expect("keyboard input should reach stdin"),
        input
    );

    let conflicting_text = RoomEnvironmentKeyboardInput::new("Früße 世界".to_string());
    let error = harness
        .dispatch_as_user(
            "owner-1",
            LocalDaemonRequest::SubmitRoomEnvironmentAction(SubmitRoomEnvironmentActionRequest {
                session_id: session.id().to_string(),
                runtime_generation: environment.runtime_generation,
                viewport_revision: environment.viewport.revision,
                idempotency_key: "keyboard-text-1".to_string(),
                action: RoomEnvironmentHumanAction::KeyboardText {
                    text: conflicting_text,
                },
            }),
        )
        .expect_err("same-length keyboard text must not share an idempotent operation identity");
    assert!(matches!(
        error,
        DaemonError::LocalTransport {
            operation: "environment.action.submit",
            message,
        } if message.starts_with("environment_idempotency_conflict:")
    ));

    let key_input = RoomEnvironmentKeyboardInput::new("ctrl+shift+p".to_string());
    let response = harness
        .dispatch_as_user(
            "owner-1",
            LocalDaemonRequest::SubmitRoomEnvironmentAction(SubmitRoomEnvironmentActionRequest {
                session_id: session.id().to_string(),
                runtime_generation: environment.runtime_generation,
                viewport_revision: environment.viewport.revision,
                idempotency_key: "keyboard-key-1".to_string(),
                action: RoomEnvironmentHumanAction::KeyboardKey {
                    key: key_input,
                    repeat: 3,
                },
            }),
        )
        .expect("valid human keyboard key chord should execute");
    let LocalDaemonResponse::RoomEnvironmentActionSubmitted {
        action_id,
        environment,
    } = response
    else {
        panic!("unexpected local response: {response:?}");
    };
    let action = environment
        .actions
        .iter()
        .find(|action| action.action_id == action_id)
        .expect("keyboard key Action should be recorded");
    assert_eq!(action.kind, "keyboard_key");
    assert_eq!(
        action.state,
        crate::session::EnvironmentActionState::Completed
    );
    assert_eq!(
        action.arguments,
        Some(crate::session::EnvironmentActionArguments::KeyboardKey { repeat: 3 })
    );
    assert_eq!(
        std::fs::read_to_string(&command_log).expect("keyboard commands should be logged"),
        "computer-type-stdin\ncomputer-key-stdin 3\n"
    );
    assert_eq!(
        std::fs::read_to_string(&input_log).expect("keyboard inputs should reach stdin"),
        "Grüße 世界ctrl+shift+p"
    );

    std::env::remove_var("CHARIOX_SLICE_SCREEN_TOOL");
    std::env::remove_var("CHARIOX_KEYBOARD_COMMAND_LOG");
    std::env::remove_var("CHARIOX_KEYBOARD_INPUT_LOG");
    std::env::remove_var("CHARIOX_SLICE_SCREEN_TOOL_TIMEOUT_MS");
    std::fs::remove_dir_all(&root).expect("test root should be removed");
}

#[test]
fn running_computer_input_cancels_the_physical_helper_and_resets_before_takeover() {
    let _guard = crate::env_lock::lock();
    let root = std::env::temp_dir().join(format!(
        "chariox-human-computer-cancellation-test-{}",
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&root).expect("test root should be created");
    let script = root.join("slice-screen.sh");
    let started = root.join("started");
    let reset = root.join("reset");
    std::fs::write(
        &script,
        "#!/bin/sh\ncase \"${1:-}\" in\n  pointer-drag)\n    : > \"$CHARIOX_COMPUTER_INPUT_STARTED\"\n    while :; do sleep 1; done\n    ;;\n  computer-input-reset)\n    : > \"$CHARIOX_COMPUTER_INPUT_RESET\"\n    ;;\nesac\n",
    )
    .expect("screen helper should be written");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))
            .expect("screen helper should be executable");
    }
    std::env::set_var("CHARIOX_SLICE_SCREEN_TOOL", &script);
    std::env::set_var("CHARIOX_COMPUTER_INPUT_STARTED", &started);
    std::env::set_var("CHARIOX_COMPUTER_INPUT_RESET", &reset);

    let harness = LocalRouterTestHarness::new();
    let (session, default_agent) = match harness
        .dispatch_as_user(
            "owner-1",
            LocalDaemonRequest::CreateSession(CreateSessionRequest::new(
                "workspace-environment-computer-cancellation",
                "worktree-environment-computer-cancellation",
            )),
        )
        .expect("Room should be created")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        other => panic!("unexpected local response: {other:?}"),
    };
    harness
        .dispatch_as_user(
            "owner-1",
            LocalDaemonRequest::StartRoomEnvironment(StartRoomEnvironmentRequest {
                session_id: session.id().to_string(),
                viewport: RoomEnvironmentViewportRequest {
                    css_width: 1280,
                    css_height: 800,
                    device_scale_factor: 1,
                    desktop_pixel_width: 1280,
                    desktop_pixel_height: 800,
                },
            }),
        )
        .expect("Room Environment should start");
    harness.with_app_mut(|app| {
        app.session_state_store()
            .transition_room_environment(session.id(), crate::session::EnvironmentLifecycle::Ready)
            .expect("Room Environment should become ready");
    });
    harness
        .dispatch_as_user(
            "owner-1",
            LocalDaemonRequest::RequestRoomEnvironmentInputTakeover(
                RequestRoomEnvironmentInputTakeoverRequest {
                    session_id: session.id().to_string(),
                    target: crate::session::InputTarget::Desktop,
                },
            ),
        )
        .expect("owner should take desktop input");

    let runtime = harness.runtime_state();
    let environment = runtime
        .room_environment_snapshot(session.id())
        .expect("Room Environment should exist");
    let session_id = session.id().to_string();
    let submit_runtime = runtime.clone();
    let submit_session_id = session_id.clone();
    let submit = harness.spawn_test_task(async move {
        submit_runtime
            .execute_human_room_environment_action(
                SubmitRoomEnvironmentActionRequest {
                    session_id: submit_session_id,
                    runtime_generation: environment.runtime_generation,
                    viewport_revision: environment.viewport.revision,
                    idempotency_key: "cancel-pointer-drag-1".to_string(),
                    action: RoomEnvironmentHumanAction::PointerDrag {
                        from_x: 120,
                        from_y: 160,
                        to_x: 720,
                        to_y: 560,
                        button: RoomEnvironmentPointerButton::Left,
                    },
                },
                crate::session::EnvironmentActor::new(
                    "user:owner-1",
                    crate::session::EnvironmentActorKind::Human,
                    "owner-1",
                ),
            )
            .await
    });

    let action_id = harness.block_on_test_task(async {
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                let snapshot = runtime
                    .room_environment_snapshot(&session_id)
                    .expect("Room Environment should remain available");
                if started.exists() {
                    if let Some(action) = snapshot
                        .actions
                        .iter()
                        .find(|action| action.kind == "pointer_drag")
                    {
                        assert_eq!(
                            action.state,
                            crate::session::EnvironmentActionState::Running
                        );
                        return action.action_id.clone();
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("physical input should start")
    });

    let cancellation = harness
        .dispatch_as_user(
            "owner-1",
            LocalDaemonRequest::CancelRoomEnvironmentAction(CancelRoomEnvironmentActionRequest {
                session_id: session_id.clone(),
                action_id: action_id.clone(),
            }),
        )
        .expect("owner should be allowed to cancel its physical input");
    let LocalDaemonResponse::RoomEnvironmentActionCancellationUpdated { outcome, .. } =
        cancellation
    else {
        panic!("unexpected local response: {cancellation:?}");
    };
    assert_eq!(
        outcome,
        crate::session::ActionCancellationOutcome::CancellationRequested
    );

    let cancellation_started = std::time::Instant::now();
    let submit_result = harness.block_on_test_task(async {
        tokio::time::timeout(std::time::Duration::from_secs(6), submit)
            .await
            .expect("cancelled physical input should finish")
            .expect("physical input task should join")
    });
    let elapsed = cancellation_started.elapsed();
    let final_environment = runtime
        .room_environment_snapshot(&session_id)
        .expect("Room Environment should remain available");
    let final_state = final_environment
        .actions
        .iter()
        .find(|action| action.action_id == action_id)
        .map(|action| action.state);
    let reset_performed = reset.exists();

    assert!(
        elapsed < std::time::Duration::from_secs(1),
        "cancellation waited for the normal helper timeout: {elapsed:?}"
    );
    let error = submit_result.expect_err("cancelled physical input must not report success");
    assert!(
        error.to_string().to_lowercase().contains("cancel"),
        "{error}"
    );
    assert_eq!(
        final_state,
        Some(crate::session::EnvironmentActionState::Cancelled)
    );
    assert!(
        reset_performed,
        "cancellation must reset held keys and buttons"
    );

    harness
        .dispatch_as_user(
            "owner-1",
            LocalDaemonRequest::ReleaseRoomEnvironmentInput(ReleaseRoomEnvironmentInputRequest {
                session_id: session_id.clone(),
                target: crate::session::InputTarget::Desktop,
            }),
        )
        .expect("owner should release desktop input before the agent acts");
    std::fs::remove_file(&started).expect("first input marker should be removed");
    std::fs::remove_file(&reset).expect("first reset marker should be removed");

    let agent_runtime = runtime.clone();
    let agent_session_id = session_id.clone();
    let agent_id = default_agent.id().to_string();
    let agent_input = harness.spawn_test_task(async move {
        agent_runtime
            .execute_computer_input_as_agent(
                &agent_session_id,
                &agent_id,
                crate::transport::room_browser_controller::RoomComputerInputAction::PointerDrag {
                    from_x: 120,
                    from_y: 160,
                    to_x: 720,
                    to_y: 560,
                    button:
                        crate::transport::room_browser_controller::RoomComputerPointerButton::Left,
                },
            )
            .await
    });
    let agent_actor_id = crate::session::agent_environment_actor_id(default_agent.id());
    let agent_action_id = harness.block_on_test_task(async {
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                let snapshot = runtime
                    .room_environment_snapshot(&session_id)
                    .expect("Room Environment should remain available");
                if started.exists() {
                    if let Some(action) = snapshot.actions.iter().find(|action| {
                        action.actor_id == agent_actor_id
                            && action.kind == "pointer_drag"
                            && action.state == crate::session::EnvironmentActionState::Running
                    }) {
                        return action.action_id.clone();
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("agent physical input should start")
    });

    let takeover_started = std::time::Instant::now();
    let takeover = harness
        .dispatch_as_user(
            "owner-1",
            LocalDaemonRequest::RequestRoomEnvironmentInputTakeover(
                RequestRoomEnvironmentInputTakeoverRequest {
                    session_id: session_id.clone(),
                    target: crate::session::InputTarget::Desktop,
                },
            ),
        )
        .expect("human takeover should request cancellation of agent Computer input");
    let LocalDaemonResponse::RoomEnvironmentTakeoverUpdated {
        outcome,
        environment,
    } = takeover
    else {
        panic!("unexpected local response: {takeover:?}");
    };
    assert_eq!(
        outcome,
        crate::session::TakeoverOutcome::CancellationRequired {
            action_ids: vec![agent_action_id.clone()],
        }
    );
    assert!(
        environment.input_ownership.is_empty(),
        "human ownership must wait until physical input is stopped and reset"
    );

    let agent_result = harness.block_on_test_task(async {
        tokio::time::timeout(std::time::Duration::from_secs(2), agent_input)
            .await
            .expect("human takeover should stop agent physical input")
            .expect("agent input task should join")
    });
    let takeover_elapsed = takeover_started.elapsed();
    let final_environment = runtime
        .room_environment_snapshot(&session_id)
        .expect("Room Environment should remain available");
    let human_actor_id = crate::session::human_environment_actor_id("owner-1");
    let agent_action = final_environment
        .actions
        .iter()
        .find(|action| action.action_id == agent_action_id)
        .expect("agent Computer Action should remain in history");
    let reset_performed = reset.exists();

    std::env::remove_var("CHARIOX_SLICE_SCREEN_TOOL");
    std::env::remove_var("CHARIOX_COMPUTER_INPUT_STARTED");
    std::env::remove_var("CHARIOX_COMPUTER_INPUT_RESET");
    std::fs::remove_dir_all(&root).expect("test root should be removed");

    assert!(
        takeover_elapsed < std::time::Duration::from_secs(1),
        "takeover waited for the normal helper timeout: {takeover_elapsed:?}"
    );
    assert!(matches!(
        agent_result,
        Err(DaemonError::BrowserControllerActionCancelled {
            controller_fenced: false
        })
    ));
    assert_eq!(
        agent_action.state,
        crate::session::EnvironmentActionState::Cancelled
    );
    assert!(reset_performed, "takeover must reset held keys and buttons");
    assert!(final_environment.input_ownership.iter().any(|ownership| {
        ownership.target == crate::session::InputTarget::Desktop
            && ownership.actor_id == human_actor_id
    }));
}

#[test]
fn queued_human_pointer_click_promotes_after_agent_action_finishes_outside_room_lane() {
    let _guard = crate::env_lock::lock();
    let root = std::env::temp_dir().join(format!(
        "chariox-queued-human-pointer-click-test-{}",
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&root).expect("test root should be created");
    let script = root.join("slice-screen.sh");
    let log = root.join("pointer-click.log");
    std::fs::write(
        &script,
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$CHARIOX_POINTER_CLICK_LOG\"\n",
    )
    .expect("screen helper should be written");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))
            .expect("screen helper should be executable");
    }
    std::env::set_var("CHARIOX_SLICE_SCREEN_TOOL", &script);
    std::env::set_var("CHARIOX_POINTER_CLICK_LOG", &log);
    std::env::set_var("CHARIOX_SLICE_SCREEN_TOOL_TIMEOUT_MS", "5000");

    let harness = LocalRouterTestHarness::new();
    let (session, agent) = match harness
        .dispatch_as_user(
            "owner-1",
            LocalDaemonRequest::CreateSession(CreateSessionRequest::new(
                "workspace-environment-queued-pointer-click",
                "worktree-environment-queued-pointer-click",
            )),
        )
        .expect("Room should be created")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        other => panic!("unexpected local response: {other:?}"),
    };
    harness
        .dispatch_as_user(
            "owner-1",
            LocalDaemonRequest::StartRoomEnvironment(StartRoomEnvironmentRequest {
                session_id: session.id().to_string(),
                viewport: RoomEnvironmentViewportRequest {
                    css_width: 1280,
                    css_height: 800,
                    device_scale_factor: 1,
                    desktop_pixel_width: 1280,
                    desktop_pixel_height: 800,
                },
            }),
        )
        .expect("Room Environment should start");
    harness.with_app_mut(|app| {
        app.session_state_store()
            .transition_room_environment(session.id(), crate::session::EnvironmentLifecycle::Ready)
            .expect("Room Environment should become ready");
        app.session_state_store()
            .reconcile_room_environment_controller_tabs(
                session.id(),
                vec![crate::session::EnvironmentTabObservation {
                    runtime_target_id: "target-a".to_string(),
                    document_id: "loader-a".to_string(),
                    url: "https://example.test".to_string(),
                    title: "Example".to_string(),
                }],
                Some("target-a"),
            )
            .expect("focused Room tab should be reconciled");
    });
    harness
        .dispatch_as_user(
            "owner-1",
            LocalDaemonRequest::RequestRoomEnvironmentInputTakeover(
                RequestRoomEnvironmentInputTakeoverRequest {
                    session_id: session.id().to_string(),
                    target: crate::session::InputTarget::Desktop,
                },
            ),
        )
        .expect("owner should take desktop input");

    let environment = harness.runtime_state();
    let snapshot = environment
        .room_environment_snapshot(session.id())
        .expect("Room Environment should exist");
    let tab_id = snapshot
        .focused_tab_id
        .clone()
        .expect("Room should have a focused tab");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let (blocker_started_tx, blocker_started_rx) = tokio::sync::oneshot::channel();
    let (release_blocker_tx, release_blocker_rx) = tokio::sync::oneshot::channel();
    let (blocker_finished_tx, blocker_finished_rx) = tokio::sync::oneshot::channel();
    let blocker_environment = environment.clone();
    let blocker_session_id = session_id.clone();
    let blocker = harness.spawn_test_task(async move {
        let result = blocker_environment
            .execute_browser_mutation_as_agent(
                &blocker_session_id,
                &agent_id,
                &tab_id,
                1,
                "blocking-agent-click",
                None,
                async move {
                    blocker_started_tx.send(()).ok();
                    release_blocker_rx
                        .await
                        .expect("blocking agent Action should release");
                    Ok::<_, DaemonError>(())
                },
            )
            .await;
        blocker_finished_tx.send(()).ok();
        result
    });
    harness.block_on_test_task(async {
        blocker_started_rx
            .await
            .expect("blocking agent Action should start")
    });

    let watcher_environment = environment.clone();
    let watcher_session_id = session_id.clone();
    let watcher = harness.spawn_test_task(async move {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
        loop {
            let snapshot = watcher_environment
                .room_environment_snapshot(&watcher_session_id)
                .expect("Room Environment should remain available");
            if snapshot.actions.iter().any(|action| {
                action.actor_id == "user:owner-1"
                    && action.state == crate::session::EnvironmentActionState::Queued
            }) {
                release_blocker_tx
                    .send(())
                    .expect("blocking agent Action should receive release");
                tokio::time::timeout(std::time::Duration::from_millis(500), blocker_finished_rx)
                    .await
                    .expect("blocking agent Action should finish outside the Room command lane")
                    .expect("blocking agent Action completion should be observed");
                return;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "human Action should enter the Room queue before its wait timeout"
            );
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    });

    let started = std::time::Instant::now();
    let response = harness
        .dispatch_as_user(
            "owner-1",
            LocalDaemonRequest::SubmitRoomEnvironmentAction(SubmitRoomEnvironmentActionRequest {
                session_id: session_id.clone(),
                runtime_generation: snapshot.runtime_generation,
                viewport_revision: snapshot.viewport.revision,
                idempotency_key: "queued-pointer-click-1".to_string(),
                action: RoomEnvironmentHumanAction::PointerClick {
                    x: 320,
                    y: 180,
                    button: RoomEnvironmentPointerButton::Left,
                    click_count: 1,
                },
            }),
        )
        .expect("queued human pointer click should execute after the agent Action finishes");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "queued human Action should promote promptly rather than reaching its five-second timeout"
    );
    let LocalDaemonResponse::RoomEnvironmentActionSubmitted {
        action_id,
        environment,
    } = response
    else {
        panic!("unexpected local response: {response:?}");
    };
    assert_eq!(
        environment
            .actions
            .iter()
            .find(|action| action.action_id == action_id)
            .map(|action| action.state),
        Some(crate::session::EnvironmentActionState::Completed)
    );
    harness.block_on_test_task(async {
        watcher.await.expect("queue watcher should join");
        blocker
            .await
            .expect("blocking task should join")
            .expect("blocking agent Action should complete");
    });
    assert_eq!(
        std::fs::read_to_string(&log).expect("physical click should be logged"),
        "pointer-click 320 180 left 1\n"
    );

    std::env::remove_var("CHARIOX_SLICE_SCREEN_TOOL");
    std::env::remove_var("CHARIOX_POINTER_CLICK_LOG");
    std::env::remove_var("CHARIOX_SLICE_SCREEN_TOOL_TIMEOUT_MS");
    std::fs::remove_dir_all(&root).expect("test root should be removed");
}

#[test]
fn room_environment_reconciles_human_and_agent_presence() {
    let harness = LocalRouterTestHarness::new();
    let (session, default_agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(
                "workspace-environment-actors",
                "worktree-environment-actors",
            ),
        ))
        .expect("Room should be created")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        other => panic!("unexpected local response: {other:?}"),
    };
    let started = harness
        .dispatch(LocalDaemonRequest::StartRoomEnvironment(
            StartRoomEnvironmentRequest {
                session_id: session.id().to_string(),
                viewport: RoomEnvironmentViewportRequest {
                    css_width: 1280,
                    css_height: 800,
                    device_scale_factor: 1,
                    desktop_pixel_width: 1280,
                    desktop_pixel_height: 800,
                },
            },
        ))
        .expect("Room Environment should start");
    let LocalDaemonResponse::RoomEnvironmentUpdated { environment } = started else {
        panic!("unexpected local response: {started:?}");
    };
    let human_actor_id =
        crate::session::human_environment_actor_id(crate::session::DEFAULT_LOCAL_USER_ID);
    let default_agent_actor_id = crate::session::agent_environment_actor_id(default_agent.id());
    assert!(environment.actors.iter().any(|actor| {
        actor.actor_id == human_actor_id
            && actor.display_label == "Local user"
            && actor.presence == crate::session::EnvironmentActorPresence::Present
    }));
    assert!(environment.actors.iter().any(|actor| {
        actor.actor_id == default_agent_actor_id
            && actor.kind == crate::session::EnvironmentActorKind::Agent
            && actor.presence == crate::session::EnvironmentActorPresence::Present
    }));

    let attachment_one = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "environment-client-1".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("first attachment should join")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        other => panic!("unexpected local response: {other:?}"),
    };
    let attachment_two = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "environment-client-2".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("second attachment should join")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        other => panic!("unexpected local response: {other:?}"),
    };
    harness
        .dispatch(LocalDaemonRequest::DetachFromSession(
            DetachFromSessionRequest {
                attachment_id: attachment_one.id().to_string(),
            },
        ))
        .expect("first attachment should leave");
    let after_first_detach = room_environment_state(&harness, session.id());
    assert_eq!(
        actor_presence(&after_first_detach, &human_actor_id),
        crate::session::EnvironmentActorPresence::Present
    );

    harness
        .dispatch(LocalDaemonRequest::DetachFromSession(
            DetachFromSessionRequest {
                attachment_id: attachment_two.id().to_string(),
            },
        ))
        .expect("second attachment should leave");
    let after_last_detach = room_environment_state(&harness, session.id());
    assert_eq!(
        actor_presence(&after_last_detach, &human_actor_id),
        crate::session::EnvironmentActorPresence::Disconnected
    );

    let spawned = match harness
        .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            account_profile: None,
            session_id: session.id().to_string(),
            alias: Some("Navigator".to_string()),
            provider: Some("dev-stub".to_string()),
            model: None,
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            kernel_ref: None,
            slice_ref: None,
            worktree_placement: None,
            metaagent: false,
        }))
        .expect("agent should spawn")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        other => panic!("unexpected local response: {other:?}"),
    };
    let spawned_actor_id = crate::session::agent_environment_actor_id(spawned.id());
    let after_spawn = room_environment_state(&harness, session.id());
    let spawned_actor = after_spawn
        .actors
        .iter()
        .find(|actor| actor.actor_id == spawned_actor_id)
        .expect("spawned agent should have an Environment Actor");
    assert_eq!(spawned_actor.display_label, "Navigator");
    assert_eq!(
        spawned_actor.presence,
        crate::session::EnvironmentActorPresence::Present
    );

    harness
        .dispatch(LocalDaemonRequest::DestroyAgent(DestroyAgentRequest {
            session_id: session.id().to_string(),
            agent_id: spawned.id().to_string(),
        }))
        .expect("agent should be destroyed");
    let after_destroy = room_environment_state(&harness, session.id());
    assert_eq!(
        actor_presence(&after_destroy, &spawned_actor_id),
        crate::session::EnvironmentActorPresence::Disconnected
    );
}

fn room_environment_state(
    harness: &LocalRouterTestHarness,
    session_id: &str,
) -> crate::session::RoomEnvironmentSnapshot {
    match harness
        .dispatch(LocalDaemonRequest::GetRoomEnvironmentState(
            GetRoomEnvironmentStateRequest {
                session_id: session_id.to_string(),
            },
        ))
        .expect("Room Environment state should be readable")
    {
        LocalDaemonResponse::RoomEnvironmentState { environment } => environment,
        other => panic!("unexpected local response: {other:?}"),
    }
}

fn actor_presence(
    environment: &crate::session::RoomEnvironmentSnapshot,
    actor_id: &str,
) -> crate::session::EnvironmentActorPresence {
    environment
        .actors
        .iter()
        .find(|actor| actor.actor_id == actor_id)
        .unwrap_or_else(|| panic!("missing Environment Actor `{actor_id}`"))
        .presence
}

#[test]
fn room_environment_viewport_update_uses_authenticated_actor_and_revision() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(
                "workspace-environment-viewport",
                "worktree-environment-viewport",
            ),
        ))
        .expect("Room should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected local response: {other:?}"),
    };
    harness
        .dispatch(LocalDaemonRequest::StartRoomEnvironment(
            StartRoomEnvironmentRequest {
                session_id: session.id().to_string(),
                viewport: RoomEnvironmentViewportRequest {
                    css_width: 1280,
                    css_height: 800,
                    device_scale_factor: 1,
                    desktop_pixel_width: 1280,
                    desktop_pixel_height: 800,
                },
            },
        ))
        .expect("Room Environment should start");
    harness.with_app_mut(|app| {
        app.session_state_store()
            .transition_room_environment(session.id(), crate::session::EnvironmentLifecycle::Ready)
            .expect("managed runtime should become ready");
    });

    let response = harness
        .dispatch(LocalDaemonRequest::UpdateRoomEnvironmentViewport(
            UpdateRoomEnvironmentViewportRequest {
                session_id: session.id().to_string(),
                expected_revision: 1,
                viewport: RoomEnvironmentViewportRequest {
                    css_width: 1440,
                    css_height: 900,
                    device_scale_factor: 2,
                    desktop_pixel_width: 2880,
                    desktop_pixel_height: 1800,
                },
            },
        ))
        .expect("authenticated Room member should update the canonical viewport");
    let LocalDaemonResponse::RoomEnvironmentUpdated { environment } = response else {
        panic!("unexpected local response: {response:?}");
    };
    let expected_actor_id =
        crate::session::human_environment_actor_id(crate::session::DEFAULT_LOCAL_USER_ID);
    assert_eq!(environment.viewport.css_width, 1440);
    assert_eq!(environment.viewport.revision, 2);
    assert_eq!(
        environment.viewport.last_actor_id.as_deref(),
        Some(expected_actor_id.as_str())
    );
    assert!(environment.actors.iter().any(|actor| {
        actor.actor_id == expected_actor_id
            && actor.kind == crate::session::EnvironmentActorKind::Human
            && actor.display_label == "Local user"
    }));

    let error = harness
        .dispatch(LocalDaemonRequest::UpdateRoomEnvironmentViewport(
            UpdateRoomEnvironmentViewportRequest {
                session_id: session.id().to_string(),
                expected_revision: 1,
                viewport: RoomEnvironmentViewportRequest {
                    css_width: 1600,
                    css_height: 1000,
                    device_scale_factor: 1,
                    desktop_pixel_width: 1600,
                    desktop_pixel_height: 1000,
                },
            },
        ))
        .expect_err("a stale viewport revision must fail");
    assert!(matches!(
        error,
        DaemonError::LocalTransport {
            operation: "environment.viewport.update",
            ..
        }
    ));
}

#[test]
fn room_environment_pointer_update_uses_authenticated_actor_and_clears_presence() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(
                "workspace-environment-pointer",
                "worktree-environment-pointer",
            ),
        ))
        .expect("Room should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected local response: {other:?}"),
    };
    harness
        .dispatch(LocalDaemonRequest::StartRoomEnvironment(
            StartRoomEnvironmentRequest {
                session_id: session.id().to_string(),
                viewport: RoomEnvironmentViewportRequest {
                    css_width: 1280,
                    css_height: 800,
                    device_scale_factor: 1,
                    desktop_pixel_width: 1280,
                    desktop_pixel_height: 800,
                },
            },
        ))
        .expect("Room Environment should start");
    harness.with_app_mut(|app| {
        app.session_state_store()
            .transition_room_environment(session.id(), crate::session::EnvironmentLifecycle::Ready)
            .expect("managed runtime should become ready");
    });

    let response = harness
        .dispatch(LocalDaemonRequest::UpdateRoomEnvironmentPointer(
            UpdateRoomEnvironmentPointerRequest {
                session_id: session.id().to_string(),
                runtime_generation: 1,
                viewport_revision: 1,
                pointer: Some(RoomEnvironmentPointerPositionRequest { x: 320, y: 180 }),
            },
        ))
        .expect("authenticated Room member should publish pointer presence");
    let LocalDaemonResponse::RoomEnvironmentUpdated { environment } = response else {
        panic!("unexpected local response: {response:?}");
    };
    let expected_actor_id =
        crate::session::human_environment_actor_id(crate::session::DEFAULT_LOCAL_USER_ID);
    assert_eq!(environment.pointers.len(), 1);
    assert_eq!(environment.pointers[0].actor_id, expected_actor_id);
    assert_eq!(
        (environment.pointers[0].x, environment.pointers[0].y),
        (320, 180)
    );
    assert!(environment.actions.is_empty());
    assert!(environment.input_ownership.is_empty());

    let error = harness
        .dispatch(LocalDaemonRequest::UpdateRoomEnvironmentPointer(
            UpdateRoomEnvironmentPointerRequest {
                session_id: session.id().to_string(),
                runtime_generation: 1,
                viewport_revision: 2,
                pointer: Some(RoomEnvironmentPointerPositionRequest { x: 321, y: 180 }),
            },
        ))
        .expect_err("a stale pointer viewport must fail closed");
    match error {
        DaemonError::LocalTransport { operation, message } => {
            assert_eq!(operation, "environment.pointer.update");
            assert!(message.starts_with("environment_stale_viewport_revision:"));
        }
        other => panic!("unexpected error: {other:?}"),
    }

    let response = harness
        .dispatch(LocalDaemonRequest::UpdateRoomEnvironmentPointer(
            UpdateRoomEnvironmentPointerRequest {
                session_id: session.id().to_string(),
                runtime_generation: 1,
                viewport_revision: 1,
                pointer: None,
            },
        ))
        .expect("mouse leave should clear authenticated pointer presence");
    let LocalDaemonResponse::RoomEnvironmentUpdated { environment } = response else {
        panic!("unexpected local response: {response:?}");
    };
    assert!(environment.pointers.is_empty());
}

#[test]
fn room_environment_start_rejects_invalid_initial_viewport_with_stable_code() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(
                "workspace-environment-invalid-viewport",
                "worktree-environment-invalid-viewport",
            ),
        ))
        .expect("Room should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected local response: {other:?}"),
    };

    let error = harness
        .dispatch(LocalDaemonRequest::StartRoomEnvironment(
            StartRoomEnvironmentRequest {
                session_id: session.id().to_string(),
                viewport: RoomEnvironmentViewportRequest {
                    css_width: 0,
                    css_height: 800,
                    device_scale_factor: 1,
                    desktop_pixel_width: 1280,
                    desktop_pixel_height: 800,
                },
            },
        ))
        .expect_err("an initial zero-width viewport must be rejected");
    match error {
        DaemonError::LocalTransport { operation, message } => {
            assert_eq!(operation, "environment.start");
            assert!(message.starts_with("environment_invalid_viewport:"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn room_environment_start_crosses_the_router_boundary_without_duplication() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-environment-start", "worktree-environment-start"),
        ))
        .expect("Room should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected local response: {other:?}"),
    };
    let request = StartRoomEnvironmentRequest {
        session_id: session.id().to_string(),
        viewport: RoomEnvironmentViewportRequest {
            css_width: 1280,
            css_height: 800,
            device_scale_factor: 2,
            desktop_pixel_width: 2560,
            desktop_pixel_height: 1600,
        },
    };

    let first = harness
        .dispatch(LocalDaemonRequest::StartRoomEnvironment(request.clone()))
        .expect("Room Environment should start through the router");
    let LocalDaemonResponse::RoomEnvironmentUpdated { environment } = first else {
        panic!("unexpected local response: {first:?}");
    };
    assert_eq!(environment.session_id, session.id());
    assert_eq!(
        environment.lifecycle,
        crate::session::EnvironmentLifecycle::Starting
    );
    assert_eq!(environment.runtime_generation, 1);
    assert_eq!(environment.viewport.css_width, 1280);

    let second = harness
        .dispatch(LocalDaemonRequest::StartRoomEnvironment(request))
        .expect("repeating start should be idempotent");
    let LocalDaemonResponse::RoomEnvironmentUpdated {
        environment: repeated,
    } = second
    else {
        panic!("unexpected local response: {second:?}");
    };
    assert_eq!(repeated.environment_id, environment.environment_id);
    assert_eq!(repeated.runtime_generation, environment.runtime_generation);
    assert_eq!(repeated.event_cursor, environment.event_cursor);

    let repeated_without_viewport = harness
        .dispatch(LocalDaemonRequest::StartRoomEnvironment(
            StartRoomEnvironmentRequest {
                session_id: session.id().to_string(),
                viewport: RoomEnvironmentViewportRequest {
                    css_width: 0,
                    css_height: 0,
                    device_scale_factor: 0,
                    desktop_pixel_width: 0,
                    desktop_pixel_height: 0,
                },
            },
        ))
        .expect("an existing Environment should keep its canonical viewport");
    let LocalDaemonResponse::RoomEnvironmentUpdated {
        environment: repeated_without_viewport,
    } = repeated_without_viewport
    else {
        panic!("unexpected local response: {repeated_without_viewport:?}");
    };
    assert_eq!(repeated_without_viewport, repeated);
}

#[test]
fn room_environment_stop_preserves_identity_and_is_idempotent() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-environment-stop", "worktree-environment-stop"),
        ))
        .expect("Room should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected local response: {other:?}"),
    };
    let started = harness
        .dispatch(LocalDaemonRequest::StartRoomEnvironment(
            StartRoomEnvironmentRequest {
                session_id: session.id().to_string(),
                viewport: RoomEnvironmentViewportRequest {
                    css_width: 1280,
                    css_height: 800,
                    device_scale_factor: 1,
                    desktop_pixel_width: 1280,
                    desktop_pixel_height: 800,
                },
            },
        ))
        .expect("Room Environment should start");
    let LocalDaemonResponse::RoomEnvironmentUpdated {
        environment: started,
    } = started
    else {
        panic!("unexpected local response: {started:?}");
    };

    let first = harness
        .dispatch(LocalDaemonRequest::StopRoomEnvironment(
            StopRoomEnvironmentRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("Room Environment should stop");
    let LocalDaemonResponse::RoomEnvironmentUpdated { environment } = first else {
        panic!("unexpected local response: {first:?}");
    };
    assert_eq!(environment.environment_id, started.environment_id);
    assert_eq!(environment.runtime_generation, started.runtime_generation);
    assert_eq!(
        environment.lifecycle,
        crate::session::EnvironmentLifecycle::Stopped
    );

    let second = harness
        .dispatch(LocalDaemonRequest::StopRoomEnvironment(
            StopRoomEnvironmentRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("repeating stop should be idempotent");
    let LocalDaemonResponse::RoomEnvironmentUpdated {
        environment: repeated,
    } = second
    else {
        panic!("unexpected local response: {second:?}");
    };
    assert_eq!(repeated, environment);

    let restarted = harness
        .dispatch(LocalDaemonRequest::StartRoomEnvironment(
            StartRoomEnvironmentRequest {
                session_id: session.id().to_string(),
                viewport: RoomEnvironmentViewportRequest {
                    css_width: 1280,
                    css_height: 800,
                    device_scale_factor: 1,
                    desktop_pixel_width: 1280,
                    desktop_pixel_height: 800,
                },
            },
        ))
        .expect("a stopped Room Environment should restart");
    let LocalDaemonResponse::RoomEnvironmentUpdated {
        environment: restarted,
    } = restarted
    else {
        panic!("unexpected local response: {restarted:?}");
    };
    assert_eq!(restarted.environment_id, environment.environment_id);
    assert_eq!(
        restarted.runtime_generation,
        environment.runtime_generation + 1
    );
    assert_eq!(
        restarted.lifecycle,
        crate::session::EnvironmentLifecycle::Starting
    );
}

#[test]
fn room_environment_retry_invalidates_failed_runtime_without_replacing_environment() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-environment-retry", "worktree-environment-retry"),
        ))
        .expect("Room should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected local response: {other:?}"),
    };
    let started = harness
        .dispatch(LocalDaemonRequest::StartRoomEnvironment(
            StartRoomEnvironmentRequest {
                session_id: session.id().to_string(),
                viewport: RoomEnvironmentViewportRequest {
                    css_width: 1280,
                    css_height: 800,
                    device_scale_factor: 1,
                    desktop_pixel_width: 1280,
                    desktop_pixel_height: 800,
                },
            },
        ))
        .expect("Room Environment should start");
    let LocalDaemonResponse::RoomEnvironmentUpdated {
        environment: started,
    } = started
    else {
        panic!("unexpected local response: {started:?}");
    };
    harness.with_app_mut(|app| {
        app.session_state_store()
            .transition_room_environment(session.id(), crate::session::EnvironmentLifecycle::Failed)
            .expect("managed runtime failure should be recorded");
    });

    let response = harness
        .dispatch(LocalDaemonRequest::RetryRoomEnvironment(
            RetryRoomEnvironmentRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("failed Room Environment should retry");
    let LocalDaemonResponse::RoomEnvironmentUpdated { environment } = response else {
        panic!("unexpected local response: {response:?}");
    };
    assert_eq!(environment.environment_id, started.environment_id);
    assert_eq!(
        environment.runtime_generation,
        started.runtime_generation + 1
    );
    assert_eq!(
        environment.lifecycle,
        crate::session::EnvironmentLifecycle::Starting
    );
}

#[test]
fn room_environment_state_crosses_the_router_boundary() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-environment", "worktree-environment"),
        ))
        .expect("Room should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected local response: {other:?}"),
    };
    harness.with_app_mut(|app| {
        app.session_state_store()
            .create_room_environment(
                session.id(),
                "environment-1",
                CanonicalViewport::new(1280, 800, 1, 1280, 800).expect("viewport should be valid"),
            )
            .expect("Room should acquire an Environment");
    });

    let response = harness
        .dispatch(LocalDaemonRequest::GetRoomEnvironmentState(
            GetRoomEnvironmentStateRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("Room Environment should be projected through the router");
    let LocalDaemonResponse::RoomEnvironmentState { environment } = response else {
        panic!("unexpected local response: {response:?}");
    };
    assert_eq!(environment.session_id, session.id());
    assert_eq!(environment.environment_id, "environment-1");
}

#[test]
fn room_environment_event_replay_crosses_the_router_boundary() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(
                "workspace-environment-events",
                "worktree-environment-events",
            ),
        ))
        .expect("Room should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected local response: {other:?}"),
    };
    harness.with_app_mut(|app| {
        let state = app.session_state_store();
        state
            .create_room_environment(
                session.id(),
                "environment-1",
                CanonicalViewport::new(1280, 800, 1, 1280, 800).expect("viewport should be valid"),
            )
            .expect("Room should acquire an Environment");
        state
            .start_room_environment(
                session.id(),
                CanonicalViewport::new(1280, 800, 1, 1280, 800).expect("viewport should be valid"),
            )
            .expect("Room Environment should start");
    });

    let response = harness
        .dispatch(LocalDaemonRequest::GetRoomEnvironmentEvents(
            GetRoomEnvironmentEventsRequest {
                session_id: session.id().to_string(),
                cursor: 0,
            },
        ))
        .expect("Room Environment events should be projected through the router");
    assert!(matches!(
        response,
        LocalDaemonResponse::RoomEnvironmentEvents {
            replay: crate::session::EnvironmentReplay::Events {
                events,
                next_cursor,
            }
        } if !events.is_empty()
            && events.windows(2).all(|pair| pair[0].event_id + 1 == pair[1].event_id)
            && next_cursor == events.last().unwrap().event_id
    ));

    let response = harness
        .dispatch(LocalDaemonRequest::GetRoomEnvironmentEvents(
            GetRoomEnvironmentEventsRequest {
                session_id: session.id().to_string(),
                cursor: u64::MAX,
            },
        ))
        .expect("a replay gap should return the authoritative Room Environment snapshot");
    assert!(matches!(
        response,
        LocalDaemonResponse::RoomEnvironmentEvents {
            replay: crate::session::EnvironmentReplay::SnapshotRequired { snapshot }
        } if snapshot.session_id == session.id()
            && snapshot.environment_id == "environment-1"
    ));
}

#[test]
fn room_environment_action_history_crosses_the_authenticated_read_boundary() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch_as_user(
            "owner-1",
            LocalDaemonRequest::CreateSession(CreateSessionRequest::new(
                "workspace-environment-history",
                "worktree-environment-history",
            )),
        )
        .expect("Room should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected local response: {other:?}"),
    };
    harness.with_app_mut(|app| {
        app.session_state_store()
            .create_room_environment(
                session.id(),
                "environment-1",
                CanonicalViewport::new(1280, 800, 1, 1280, 800).expect("viewport should be valid"),
            )
            .expect("Room should acquire an Environment");
    });

    let response = harness
        .dispatch_as_user(
            "owner-1",
            LocalDaemonRequest::ListRoomEnvironmentActionHistory(
                crate::local::ListRoomEnvironmentActionHistoryRequest {
                    session_id: session.id().to_string(),
                    before_sequence: None,
                    limit: Some(25),
                },
            ),
        )
        .expect("Room members should list Environment Action history");
    assert!(matches!(
        response,
        LocalDaemonResponse::RoomEnvironmentActionHistoryListed { page }
            if page.actions.is_empty() && page.next_before_sequence.is_none()
    ));

    let error = harness
        .dispatch_as_user(
            "outsider-1",
            LocalDaemonRequest::ListRoomEnvironmentActionHistory(
                crate::local::ListRoomEnvironmentActionHistoryRequest {
                    session_id: session.id().to_string(),
                    before_sequence: None,
                    limit: Some(25),
                },
            ),
        )
        .expect_err("an outsider must not list Environment Action history");
    assert!(matches!(error, DaemonError::SessionAccessDenied { .. }));
}

#[test]
fn room_environment_state_requires_room_membership() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch_as_user(
            "owner-1",
            LocalDaemonRequest::CreateSession(CreateSessionRequest::new(
                "workspace-environment-auth",
                "worktree-environment-auth",
            )),
        )
        .expect("Room should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected local response: {other:?}"),
    };

    let error = harness
        .dispatch_as_user(
            "outsider-1",
            LocalDaemonRequest::GetRoomEnvironmentState(GetRoomEnvironmentStateRequest {
                session_id: session.id().to_string(),
            }),
        )
        .expect_err("an outsider must not read the Room Environment");
    assert!(matches!(error, DaemonError::SessionAccessDenied { .. }));

    let error = harness
        .dispatch_as_user(
            "outsider-1",
            LocalDaemonRequest::GetRoomEnvironmentEvents(GetRoomEnvironmentEventsRequest {
                session_id: session.id().to_string(),
                cursor: 0,
            }),
        )
        .expect_err("an outsider must not replay Room Environment events");
    assert!(matches!(error, DaemonError::SessionAccessDenied { .. }));
}

#[test]
fn room_environment_lifecycle_requires_room_membership() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch_as_user(
            "owner-1",
            LocalDaemonRequest::CreateSession(CreateSessionRequest::new(
                "workspace-environment-start-auth",
                "worktree-environment-start-auth",
            )),
        )
        .expect("Room should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected local response: {other:?}"),
    };

    let error = harness
        .dispatch_as_user(
            "outsider-1",
            LocalDaemonRequest::StartRoomEnvironment(StartRoomEnvironmentRequest {
                session_id: session.id().to_string(),
                viewport: RoomEnvironmentViewportRequest {
                    css_width: 1280,
                    css_height: 800,
                    device_scale_factor: 1,
                    desktop_pixel_width: 1280,
                    desktop_pixel_height: 800,
                },
            }),
        )
        .expect_err("an outsider must not start the Room Environment");
    assert!(matches!(error, DaemonError::SessionAccessDenied { .. }));

    for request in [
        LocalDaemonRequest::StopRoomEnvironment(StopRoomEnvironmentRequest {
            session_id: session.id().to_string(),
        }),
        LocalDaemonRequest::RetryRoomEnvironment(RetryRoomEnvironmentRequest {
            session_id: session.id().to_string(),
        }),
        LocalDaemonRequest::RequestRoomEnvironmentInputTakeover(
            RequestRoomEnvironmentInputTakeoverRequest {
                session_id: session.id().to_string(),
                target: crate::session::InputTarget::Desktop,
            },
        ),
        LocalDaemonRequest::ReleaseRoomEnvironmentInput(ReleaseRoomEnvironmentInputRequest {
            session_id: session.id().to_string(),
            target: crate::session::InputTarget::Desktop,
        }),
        LocalDaemonRequest::CancelRoomEnvironmentAction(CancelRoomEnvironmentActionRequest {
            session_id: session.id().to_string(),
            action_id: "action-1".to_string(),
        }),
        LocalDaemonRequest::SubmitRoomEnvironmentAction(SubmitRoomEnvironmentActionRequest {
            session_id: session.id().to_string(),
            runtime_generation: 1,
            viewport_revision: 1,
            idempotency_key: "input-1".to_string(),
            action: RoomEnvironmentHumanAction::PointerClick {
                x: 10,
                y: 10,
                button: RoomEnvironmentPointerButton::Left,
                click_count: 1,
            },
        }),
    ] {
        let error = harness
            .dispatch_as_user("outsider-1", request)
            .expect_err("an outsider must not control the Room Environment lifecycle");
        assert!(matches!(error, DaemonError::SessionAccessDenied { .. }));
    }
}
