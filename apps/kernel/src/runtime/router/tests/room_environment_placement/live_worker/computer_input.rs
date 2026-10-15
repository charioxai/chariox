use super::*;
use futures_util::FutureExt;

#[test]
fn bound_worker_applies_authenticated_mouse_and_keyboard_input_without_a_browser_controller() {
    crate::test_support::isolated_env_test!();
    run_test(applies_authenticated_mouse_and_keyboard_input_without_a_browser_controller);
}

async fn applies_authenticated_mouse_and_keyboard_input_without_a_browser_controller() {
    let _guard = crate::env_lock::lock();
    let mut worker_state = TestState::new();
    let home = DaemonConfig::for_tests();
    worker_state.config.host_machine_id = "slice:slice-1".to_string();
    worker_state.config.room_environment_worker_binding =
        Some(crate::config::RoomEnvironmentWorkerBinding {
            provisioned_slice_id: None,
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
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$CHARIOX_COMPUTER_INPUT_COMMAND_LOG\"\ncase \"${1:-}\" in computer-type-stdin|computer-key-stdin) cat >> \"$CHARIOX_COMPUTER_INPUT_STDIN_LOG\" ;; esac\nif [ \"${1:-}\" = computer-type-stdin ]; then sleep 6; fi\n",
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
                    "Grüße 世界 ".repeat(20),
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
        format!("{}ctrl+shift+p", "Grüße 世界 ".repeat(20))
    );

    std::env::remove_var("CHARIOX_SLICE_SCREEN_TOOL");
    std::env::remove_var("CHARIOX_COMPUTER_INPUT_COMMAND_LOG");
    std::env::remove_var("CHARIOX_COMPUTER_INPUT_STDIN_LOG");
}

#[test]
fn room_environment_cancels_worker_computer_input_over_the_relay_before_takeover() {
    crate::test_support::isolated_env_test!();
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
        "#!/bin/sh\ncase \"${1:-}\" in\n  pointer-click)\n    : > \"$CHARIOX_COMPUTER_INPUT_STARTED\"\n    while :; do sleep 1; done\n    ;;\n  computer-input-reset)\n    : > \"$CHARIOX_COMPUTER_INPUT_RESET\"\n    ;;\nesac\n",
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
                    crate::transport::room_browser_controller::RoomComputerInputAction::PointerClick {
                        x: 120,
                        y: 160,
                        click_count: 1,
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
                action["kind"] == "pointer_click" && action["state"] == "cancelled"
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

// MP-08 / MP-11: the worker owns capture exclusion for its physical display.
#[test]
fn efix5_computer_secret_withholds_screenshots_and_ocr_during_input() {
    crate::test_support::isolated_env_test!();
    run_test(withholds_capture_during_secret_input);
}

async fn withholds_capture_during_secret_input() {
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
            provisioned_slice_id: None,
        });
    std::fs::create_dir_all(&worker_state.root).unwrap();
    let script = worker_state.root.join("secret-screen.sh");
    let started = worker_state.root.join("input-started");
    let captures = worker_state.root.join("capture-called");
    let fixture = worker_state.root.join("safe.png");
    std::fs::write(&fixture, b"\x89PNG\r\n\x1a\nsafe-fixture").unwrap();
    std::fs::write(&script, format!(r#"#!/bin/sh
set -eu
case "$1" in
computer-secret-paste-stdin) cat >/dev/null; touch '{}'; sleep 1 ;;
protected-*)
  python3 -c 'import json,sys; p=json.load(sys.stdin); assert not p["unknown"]; assert p["values"] == ["synthetic"]; assert p["targets"][0]["kind"] == "native"; assert p["targets"][0]["target"]["focus_window"] == 101'
  touch '{}'
  case "$1" in
    protected-screenshot) cp '{}' "$2" ;;
    protected-ocr) printf safe ;;
    protected-find-text) printf null ;;
  esac ;;
*) exit 2 ;;
esac
"#, started.display(), captures.display(), fixture.display())).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    std::env::set_var("CHARIOX_SLICE_SCREEN_TOOL", &script);
    let (worker, _) = worker_state.router();
    let action = serde_json::from_value(serde_json::json!({
        "kind":"secret_text", "input":"synthetic",
        "expected_target":{"focus_window":101,"active_window":100,
            "geometry":[20,30,200,40],"window_geometry":[0,0,800,600]}
    }))
    .unwrap();
    let input = worker.relay_room_browser_controller(
        "home-kernel",
        &home.relay_public_key,
        "room-1",
        "slice-1",
        crate::transport::room_browser_controller::RoomBrowserControllerCommand::ComputerInput {
            action_id: "secret-1".into(),
            actor_id: "agent:agent-1".into(),
            runtime_generation: 1,
            viewport_revision: 1,
            desktop_pixel_width: 1280,
            desktop_pixel_height: 800,
            action,
        },
    );
    let observe = async {
        tokio::time::timeout(Duration::from_secs(10), async {
            while !started.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let screenshot = worker.relay_capture_room_screenshot(
            "home-kernel",
            &home.relay_public_key,
            "room-1",
            "slice-1",
        );
        let ocr = worker.relay_observe_room_computer(
            "home-kernel",
            &home.relay_public_key,
            "room-1",
            "slice-1",
            crate::transport::relay_peer::RemoteRoomComputerObservationCall::Ocr {
                artifact_id: None,
            },
        );
        let frame_text = worker.relay_observe_room_computer(
            "home-kernel",
            &home.relay_public_key,
            "room-1",
            "slice-1",
            crate::transport::relay_peer::RemoteRoomComputerObservationCall::FindText {
                query: "synthetic".into(),
                artifact_id: None,
            },
        );
        let during_input = async {
            tokio::time::sleep(Duration::from_millis(200)).await;
            assert!(
                !captures.exists(),
                "capture reached the display helper during insertion"
            );
        };
        let (screenshot, ocr, frame_text, ()) =
            tokio::join!(screenshot, ocr, frame_text, during_input);
        (screenshot, ocr, frame_text)
    };
    let (input_result, (screenshot, ocr, frame_text)) = tokio::join!(input, observe);
    let resumed = worker
        .relay_capture_room_screenshot("home-kernel", &home.relay_public_key, "room-1", "slice-1")
        .await;
    std::env::remove_var("CHARIOX_SLICE_SCREEN_TOOL");
    input_result.expect("secret input should settle");
    // The observation barrier releases only after insertion. Every resumed
    // capture uses the protected helper and its registered native mask policy.
    screenshot.expect("fresh protected screenshot resumes after insertion");
    assert!(ocr.expect("protected OCR resumes after insertion").ok);
    assert!(
        frame_text
            .expect("protected text lookup resumes after insertion")
            .ok
    );
    assert!(
        captures.exists(),
        "protected capture should resume autonomously"
    );
    resumed.expect("capture resumes after insertion");
}

// MP-08 / MP-11: authenticated worker route retains agent admission and human/Vault paths.
#[test]
fn mp11_room_worker_keyboard_carries_native_admission_and_typed_refusal() {
    crate::test_support::isolated_env_test!();
    run_test(room_worker_native_input_admission);
}

async fn room_worker_native_input_admission() {
    let _guard = crate::env_lock::lock();
    let mut worker_state = TestState::new();
    let home = DaemonConfig::for_tests();
    worker_state.config.host_machine_id = "slice:slice-1".into();
    worker_state.config.room_environment_worker_binding =
        Some(crate::config::RoomEnvironmentWorkerBinding {
            home_kernel_id: "home-kernel".into(),
            home_public_key: home.relay_public_key.clone(),
            session_id: "room-1".into(),
            slice_id: "slice-1".into(),
            provisioned_slice_id: None,
        });
    std::fs::create_dir_all(&worker_state.root).unwrap();
    let script = worker_state.root.join("native-input-admission.sh");
    let effects = worker_state.root.join("human-effects");
    // Supplementary transport regression: the real native focus oracle lives in Python/live drills.
    std::fs::write(
        &script,
        format!(
            r#"#!/bin/sh
set -eu
cat >/dev/null
if [ "${{CHARIOX_COMPUTER_AGENT_INPUT:-}}" = 1 ]; then
  printf 'user_domain_sensitive_requires_focus\n' >&2
  exit 1
fi
printf human >> '{}'
"#,
            effects.display()
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    std::env::set_var("CHARIOX_SLICE_SCREEN_TOOL", &script);
    let (worker, _) = worker_state.router();
    for (index, action) in [
        crate::transport::room_browser_controller::RoomComputerInputAction::KeyboardText {
            input: crate::transport::room_browser_controller::RoomComputerKeyboardInput::new(
                "public".into(),
            ),
        },
        crate::transport::room_browser_controller::RoomComputerInputAction::KeyboardKey {
            input: crate::transport::room_browser_controller::RoomComputerKeyboardInput::new(
                "a".into(),
            ),
            repeat: 2,
        },
    ]
    .into_iter()
    .enumerate()
    {
        for actor in ["agent:agent-1", "user:owner-1"] {
            // The kernel must replace stale inherited markers for either actor.
            std::env::set_var(
                "CHARIOX_COMPUTER_AGENT_INPUT",
                if actor.starts_with("agent:") {
                    "0"
                } else {
                    "1"
                },
            );
            let result = worker.relay_room_browser_controller(
                "home-kernel", &home.relay_public_key, "room-1", "slice-1",
                crate::transport::room_browser_controller::RoomBrowserControllerCommand::ComputerInput {
                    action_id: format!("admit-{index}-{actor}"), actor_id: actor.into(), runtime_generation: 1,
                    viewport_revision: 1, desktop_pixel_width: 1280, desktop_pixel_height: 800, action: action.clone(),
                },
            ).await;
            if actor.starts_with("agent:") {
                let error = result.expect_err("agent native guard must refuse the protected focus");
                assert!(matches!(
                    error,
                    DaemonError::UserDomainRefused {
                        reason: crate::error::UserDomainRefusalReason::SensitiveRequiresFocus
                    }
                ));
            } else {
                result.expect("human input retains its existing path");
            }
        }
    }
    assert_eq!(std::fs::read_to_string(&effects).unwrap(), "humanhuman");
    std::env::remove_var("CHARIOX_COMPUTER_AGENT_INPUT");
    std::env::remove_var("CHARIOX_SLICE_SCREEN_TOOL");
    std::fs::remove_dir_all(&worker_state.root).unwrap();
}

// MP-08 / MP-11 R1: the authenticated Room worker refuses unfenced agent
// mutations before invoking even a helper that would blindly emit an event.
#[test]
fn mp11_room_worker_unfenced_agent_actions_never_reach_physical_helper() {
    crate::test_support::isolated_env_test!();
    run_test(room_worker_unfenced_agent_actions);
}

async fn room_worker_unfenced_agent_actions() {
    let _guard = crate::env_lock::lock();
    let mut state = TestState::new();
    let home = DaemonConfig::for_tests();
    state.config.host_machine_id = "slice:slice-1".into();
    state.config.room_environment_worker_binding =
        Some(crate::config::RoomEnvironmentWorkerBinding {
            home_kernel_id: "home-kernel".into(),
            home_public_key: home.relay_public_key.clone(),
            session_id: "room-1".into(),
            slice_id: "slice-1".into(),
            provisioned_slice_id: None,
        });
    std::fs::create_dir_all(&state.root).unwrap();
    let script = state.root.join("blind-native-helper.sh");
    let effects = state.root.join("physical-events");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\ncat >/dev/null\nprintf event >> '{}'\n",
            effects.display()
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    std::env::set_var("CHARIOX_SLICE_SCREEN_TOOL", &script);
    let (worker, _) = state.router();
    use crate::transport::room_browser_controller::{
        RoomComputerInputAction as A, RoomComputerKeyboardInput as K,
        RoomComputerPointerButton as B,
    };
    let actions = [
        A::KeyboardHold {
            input: K::new("a".into()),
            duration_ms: 100,
        },
        A::PointerHold {
            x: 10,
            y: 10,
            button: B::Left,
            duration_ms: 100,
        },
        A::ClipboardWrite {
            text: crate::transport::room_browser_controller::RoomComputerClipboardText::new(
                "public".into(),
            ),
        },
        A::PointerDrag {
            from_x: 10,
            from_y: 10,
            to_x: 20,
            to_y: 20,
            button: B::Left,
        },
        A::PointerClick {
            x: 10,
            y: 10,
            button: B::Middle,
            click_count: 1,
        },
    ];
    for (index, action) in actions.into_iter().enumerate() {
        for actor in ["agent:agent-1", "user:owner-1"] {
            let prior = std::fs::read(&effects).unwrap_or_default();
            let result=worker.relay_room_browser_controller("home-kernel", &home.relay_public_key,"room-1","slice-1",
                crate::transport::room_browser_controller::RoomBrowserControllerCommand::ComputerInput {
                    action_id:format!("unfenced-{index}-{actor}"), actor_id:actor.into(), runtime_generation:1, viewport_revision:1,
                    desktop_pixel_width:1280,desktop_pixel_height:800,action:action.clone(),
                }).await;
            if actor.starts_with("agent:") {
                assert!(
                    matches!(
                        result,
                        Err(DaemonError::UserDomainRefused {
                            reason: crate::error::UserDomainRefusalReason::SensitiveRequiresFocus
                        })
                    ),
                    "agent mutation must require human or approved Vault input: {result:?}"
                );
                assert_eq!(
                    std::fs::read(&effects).unwrap_or_default(),
                    prior,
                    "refused input reached the physical helper"
                );
            } else {
                result.expect("human mutation retains its admitted path");
            }
        }
    }
    assert_eq!(
        std::fs::read_to_string(&effects).unwrap(),
        "event".repeat(5)
    );
    std::env::remove_var("CHARIOX_SLICE_SCREEN_TOOL");
    std::fs::remove_dir_all(&state.root).unwrap();
}
