use super::*;

#[test]
fn bound_worker_applies_authenticated_computer_input_without_a_browser_controller() {
    run_test(applies_authenticated_computer_input_without_a_browser_controller);
}

async fn applies_authenticated_computer_input_without_a_browser_controller() {
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
    let log = worker_state.root.join("pointer-click.log");
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
    let (worker, _) = worker_state.router();
    let command =
        crate::transport::room_browser_controller::RoomBrowserControllerCommand::ComputerInput {
            action_id: "action-1".to_string(),
            actor_id: "user:owner-1".to_string(),
            runtime_generation: 1,
            viewport_revision: 1,
            desktop_pixel_width: 1280,
            desktop_pixel_height: 800,
            action:
                crate::transport::room_browser_controller::RoomComputerInputAction::PointerClick {
                    x: 400,
                    y: 240,
                    button:
                        crate::transport::room_browser_controller::RoomComputerPointerButton::Middle,
                    click_count: 1,
                },
        };

    let denied = worker
        .relay_room_browser_controller(
            "wrong-home",
            &home.relay_public_key,
            "room-1",
            "slice-1",
            command.clone(),
        )
        .await
        .expect_err("a mismatched home kernel must be rejected");
    assert!(denied
        .to_string()
        .contains("browser_controller_scope_denied"));
    assert!(!log.exists(), "denied input must not reach the helper");

    let result = worker
        .relay_room_browser_controller(
            "home-kernel",
            &home.relay_public_key,
            "room-1",
            "slice-1",
            command,
        )
        .await
        .expect("the provisioned home should apply Computer input");
    assert_eq!(
        result,
        crate::transport::room_browser_controller::RoomBrowserControllerResult::ComputerInputApplied {
            action_id: "action-1".to_string(),
        }
    );
    assert_eq!(
        std::fs::read_to_string(&log).expect("worker click should be logged"),
        "pointer-click 400 240 middle 1\n"
    );

    std::env::remove_var("CHARIOX_SLICE_SCREEN_TOOL");
    std::env::remove_var("CHARIOX_POINTER_CLICK_LOG");
}
