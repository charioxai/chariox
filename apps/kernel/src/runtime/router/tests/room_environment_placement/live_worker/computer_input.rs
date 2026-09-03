use super::*;
use futures_util::FutureExt;

#[test]
fn bound_worker_applies_authenticated_mouse_and_keyboard_input_without_a_browser_controller() {
    run_test(applies_authenticated_mouse_and_keyboard_input_without_a_browser_controller);
}

async fn applies_authenticated_mouse_and_keyboard_input_without_a_browser_controller() {
    let _guard = crate::env_lock::lock();
    let mut worker_state = TestState::new();
    let home = DaemonConfig::for_tests();
    worker_state.config.host_machine_id = "slice:slice-1".to_string();
    worker_state.config.room_environment_worker_binding =
        Some(crate::config::RoomEnvironmentWorkerBinding {
            home_kernel_id: "home-kernel".to_string(),
            home_public_key: home.relay_public_key.clone(),
            session_id: "room-1".to_string(),
            slice_id: "slice-1".to_string(),
        });
    std::fs::create_dir_all(&worker_state.root).expect("worker state root should be created");
    let script = worker_state.root.join("slice-screen.sh");
    let command_log = worker_state.root.join("computer-input-command.log");
    let input_log = worker_state.root.join("computer-input-stdin.log");
    std::fs::write(
        &script,
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$CHARIOX_COMPUTER_INPUT_COMMAND_LOG\"\ncase \"${1:-}\" in computer-type-stdin|computer-key-stdin) cat >> \"$CHARIOX_COMPUTER_INPUT_STDIN_LOG\" ;; esac\n",
    )
    .expect("screen helper should be written");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))
            .expect("screen helper should be executable");
    }
    std::env::set_var("CHARIOX_SLICE_SCREEN_TOOL", &script);
    std::env::set_var("CHARIOX_COMPUTER_INPUT_COMMAND_LOG", &command_log);
    std::env::set_var("CHARIOX_COMPUTER_INPUT_STDIN_LOG", &input_log);
    let (worker, _) = worker_state.router();
    let command =
        |action_id: &str,
         action: crate::transport::room_browser_controller::RoomComputerInputAction| {
            crate::transport::room_browser_controller::RoomBrowserControllerCommand::ComputerInput {
                action_id: action_id.to_string(),
                actor_id: "user:owner-1".to_string(),
                runtime_generation: 1,
                viewport_revision: 1,
                desktop_pixel_width: 1280,
                desktop_pixel_height: 800,
                action,
            }
        };
    let click = crate::transport::room_browser_controller::RoomComputerInputAction::PointerClick {
        x: 400,
        y: 240,
        button: crate::transport::room_browser_controller::RoomComputerPointerButton::Middle,
        click_count: 1,
    };

    let denied = worker
        .relay_room_browser_controller(
            "wrong-home",
            &home.relay_public_key,
            "room-1",
            "slice-1",
            command("denied-click", click.clone()),
        )
        .await
        .expect_err("a mismatched home kernel must be rejected");
    assert!(denied
        .to_string()
        .contains("browser_controller_scope_denied"));
    assert!(
        !command_log.exists(),
        "denied input must not reach the helper"
    );

    let actions = [
        ("click", click),
        (
            "drag",
            crate::transport::room_browser_controller::RoomComputerInputAction::PointerDrag {
                from_x: 120,
                from_y: 160,
                to_x: 720,
                to_y: 560,
                button: crate::transport::room_browser_controller::RoomComputerPointerButton::Left,
            },
        ),
        (
            "scroll",
            crate::transport::room_browser_controller::RoomComputerInputAction::PointerScroll {
                x: 640,
                y: 400,
                horizontal_steps: -3,
                vertical_steps: 5,
            },
        ),
        (
            "text",
            crate::transport::room_browser_controller::RoomComputerInputAction::KeyboardText {
                input: crate::transport::room_browser_controller::RoomComputerKeyboardInput::new(
                    "Grüße 世界".to_string(),
                ),
            },
        ),
        (
            "key",
            crate::transport::room_browser_controller::RoomComputerInputAction::KeyboardKey {
                input: crate::transport::room_browser_controller::RoomComputerKeyboardInput::new(
                    "ctrl+shift+p".to_string(),
                ),
                repeat: 3,
            },
        ),
    ];
    for (action_id, action) in actions {
        let result = worker
            .relay_room_browser_controller(
                "home-kernel",
                &home.relay_public_key,
                "room-1",
                "slice-1",
                command(action_id, action),
            )
            .await
            .expect("the provisioned home should apply Computer input");
        assert_eq!(
            result,
            crate::transport::room_browser_controller::RoomBrowserControllerResult::ComputerInputApplied {
                action_id: action_id.to_string(),
            }
        );
    }
    assert_eq!(
        std::fs::read_to_string(&command_log).expect("worker input commands should be logged"),
        concat!(
            "pointer-click 400 240 middle 1\n",
            "pointer-drag 120 160 720 560 left\n",
            "pointer-scroll 640 400 -3 5\n",
            "computer-type-stdin\n",
            "computer-key-stdin 3\n",
        )
    );
    assert_eq!(
        std::fs::read_to_string(&input_log).expect("worker keyboard input should reach stdin"),
        "Grüße 世界ctrl+shift+p"
    );

    std::env::remove_var("CHARIOX_SLICE_SCREEN_TOOL");
    std::env::remove_var("CHARIOX_COMPUTER_INPUT_COMMAND_LOG");
    std::env::remove_var("CHARIOX_COMPUTER_INPUT_STDIN_LOG");
}

#[test]
fn room_environment_cancels_worker_computer_input_over_the_relay_before_takeover() {
    run_test(cancels_worker_computer_input_over_the_relay_before_takeover);
}

async fn cancels_worker_computer_input_over_the_relay_before_takeover() {
    let _guard = crate::env_lock::lock();
    let root = std::env::temp_dir().join(format!(
        "chariox-worker-computer-cancellation-test-{}",
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

    let mut fixture = LiveWorker::start_configured(false, true).await;
    let assertions = std::panic::AssertUnwindSafe(async {
        fixture.create_slice().await;
        fixture
            .home
            .app
            .lock()
            .await
            .slices()
            .set_status(
                "desktop",
                crate::slice::SliceStatus::Running,
                crate::session::unix_epoch_ms(),
            )
            .expect("fixture slice should be running");
        let room = fixture.rooms[0].clone();
        dispatch_json(
            &fixture.home,
            json!({"BindRoomEnvironmentSlice": {
                "session_id":room, "slice_ref":"desktop"
            }}),
        )
        .await
        .expect("Room should bind to its worker slice");
        dispatch_json(
            &fixture.home,
            json!({"StartRoomEnvironment": {
                "session_id":room, "viewport": {
                    "css_width":1280, "css_height":800, "device_scale_factor":1,
                    "desktop_pixel_width":1280, "desktop_pixel_height":800
                }
            }}),
        )
        .await
        .expect("Room Environment should start through the bound worker");
        let listed = dispatch_json(
            &fixture.home,
            json!({"ListAgents":{"session_id":room}}),
        )
        .await
        .expect("Room agent should be listed");
        let agent_id = listed["AgentsListed"]["agents"][0]["id"]
            .as_str()
            .expect("default agent id")
            .to_string();
        let runtime = fixture.home.runtime_state.clone();
        let action_room = room.clone();
        let action = tokio::spawn(async move {
            runtime
                .execute_computer_input_as_agent(
                    &action_room,
                    &agent_id,
                    crate::transport::room_browser_controller::RoomComputerInputAction::PointerDrag {
                        from_x: 120,
                        from_y: 160,
                        to_x: 720,
                        to_y: 560,
                        button: crate::transport::room_browser_controller::RoomComputerPointerButton::Left,
                    },
                )
                .await
        });
        timeout(Duration::from_secs(2), async {
            while !started.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("physical input should start on the worker");

        let takeover_started = std::time::Instant::now();
        let takeover = dispatch_json(
            &fixture.home,
            json!({"RequestRoomEnvironmentInputTakeover": {
                "session_id":room, "target":{"kind":"desktop"}
            }}),
        )
        .await
        .expect("human takeover should cancel worker Computer input");
        assert_eq!(
            takeover["RoomEnvironmentTakeoverUpdated"]["outcome"]["state"],
            "cancellation_required"
        );
        let action_result = timeout(Duration::from_secs(3), action)
            .await
            .expect("worker Computer input should stop")
            .expect("worker Computer input task should join");
        assert!(
            takeover_started.elapsed() < Duration::from_secs(1),
            "takeover must not wait for the worker helper timeout"
        );
        assert!(matches!(
            action_result,
            Err(DaemonError::BrowserControllerActionCancelled {
                controller_fenced: false
            })
        ));
        assert!(reset.exists(), "worker must reset physical input");

        let state = dispatch_json(
            &fixture.home,
            json!({"GetRoomEnvironmentState":{"session_id":room}}),
        )
        .await
        .expect("Room Environment state should remain readable");
        let environment = &state["RoomEnvironmentState"]["environment"];
        assert!(environment["actions"].as_array().is_some_and(|actions| {
            actions.iter().any(|action| {
                action["kind"] == "pointer_drag" && action["state"] == "cancelled"
            })
        }));
        assert!(environment["input_ownership"]
            .as_array()
            .is_some_and(|owners| owners.iter().any(|owner| {
                owner["target"]["kind"] == "desktop"
                    && owner["actor_id"]
                        .as_str()
                        .is_some_and(|actor| actor.starts_with("user:"))
            })));
    })
    .catch_unwind()
    .await;

    let controller_cleanup = fixture
        .worker
        .runtime_state
        .shutdown_browser_controller_process()
        .await;
    fixture.stop().await;
    std::env::remove_var("CHARIOX_SLICE_SCREEN_TOOL");
    std::env::remove_var("CHARIOX_COMPUTER_INPUT_STARTED");
    std::env::remove_var("CHARIOX_COMPUTER_INPUT_RESET");
    std::fs::remove_dir_all(&root).expect("test root should be removed");
    controller_cleanup.expect("fixture controller should stop");
    if let Err(panic) = assertions {
        std::panic::resume_unwind(panic);
    }
}
