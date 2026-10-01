use super::*;
use futures_util::FutureExt;

#[test]
fn human_room_input_waits_for_controller_route_and_has_one_effect() {
    run_test(input_waits_for_controller_route);
}

async fn input_waits_for_controller_route() {
    contended_input(WaitingInput::Complete).await;
}

#[test]
fn human_room_input_cancelled_while_waiting_has_no_effect() {
    run_test(cancelled_input_waits_for_controller_route);
}

async fn cancelled_input_waits_for_controller_route() {
    contended_input(WaitingInput::Cancel).await;
}

#[test]
fn human_room_input_with_changed_viewport_while_waiting_has_no_effect() {
    run_test(stale_input_waits_for_controller_route);
}

async fn stale_input_waits_for_controller_route() {
    contended_input(WaitingInput::ChangeViewport).await;
}

#[derive(Clone, Copy)]
enum WaitingInput {
    Complete,
    Cancel,
    ChangeViewport,
}

async fn contended_input(outcome: WaitingInput) {
    let mut fixture = LiveWorker::start_configured(false, true).await;
    let log = fixture._worker_state.root.join("input-effects.log");
    let helper = fixture._worker_state.root.join("input-helper.sh");
    std::fs::write(
        &helper,
        format!("#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\ncase \"$1\" in computer-key-stdin) cat >/dev/null ;; esac\n", log.display()),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::env::set_var("CHARIOX_SLICE_SCREEN_TOOL", &helper);
    let assertions = std::panic::AssertUnwindSafe(async {
        fixture.create_slice().await;
        let slices = fixture.home.app.lock().await.slices().clone();
        slices
            .set_status("desktop", SliceStatus::Running, 1)
            .unwrap();
        let room = &fixture.rooms[0];
        dispatch_json(&fixture.home, bind(room, "desktop"))
            .await
            .unwrap();
        let started = dispatch_json(
            &fixture.home,
            json!({"StartRoomEnvironment": {
                "session_id":room, "viewport": {
                    "css_width":1280,"css_height":800,"device_scale_factor":1,
                    "desktop_pixel_width":1280,"desktop_pixel_height":800
                }
            }}),
        )
        .await
        .unwrap();
        let environment = &started["RoomEnvironmentUpdated"]["environment"];
        dispatch_json(
            &fixture.home,
            json!({"RequestRoomEnvironmentInputTakeover": {
                "session_id":room,"target":{"kind":"desktop"}
            }}),
        )
        .await
        .unwrap();
        let request = json!({"SubmitRoomEnvironmentAction": {
            "session_id":room,"runtime_generation":environment["runtime_generation"],
            "viewport_revision":environment["viewport"]["revision"],
            "idempotency_key":"contended-human-click",
            "action":{"kind":"pointer_click","x":320,"y":180,"button":"left","click_count":1}
        }});
        let route = slices
            .guard_environment_use("desktop", Some(room), "browser_controller.route")
            .unwrap();
        let runtime = fixture.home.runtime_state.clone();
        let action_room = room.clone();
        let release = tokio::spawn(async move {
            timeout(Duration::from_secs(2), async {
                loop {
                    let environment = runtime.room_environment_snapshot(&action_room).unwrap();
                    if let Some(action) = environment
                        .actions
                        .iter()
                        .find(|action| action.kind == "pointer_click")
                    {
                        if matches!(outcome, WaitingInput::Cancel) {
                            runtime
                                .cancel_room_environment_action_as_actor(
                                    &action_room,
                                    crate::session::EnvironmentActor::new(
                                        action.actor_id.clone(),
                                        crate::session::EnvironmentActorKind::Human,
                                        "viewer",
                                    ),
                                    &action.action_id,
                                )
                                .unwrap();
                        } else if matches!(outcome, WaitingInput::ChangeViewport) {
                            let mut viewport = environment.viewport.clone();
                            viewport.css_width += 1;
                            runtime
                                .update_room_environment_viewport_as_actor(
                                    &action_room,
                                    crate::session::EnvironmentActor::new(
                                        action.actor_id.clone(),
                                        crate::session::EnvironmentActorKind::Human,
                                        "viewer",
                                    ),
                                    environment.viewport.revision,
                                    viewport,
                                )
                                .unwrap();
                        }
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("human Action should enter the ledger while the route is occupied");
            tokio::time::sleep(Duration::from_millis(100)).await;
            drop(route);
        });
        let read = json!({"GetRoomEnvironmentTabAccessibility": {
            "session_id":room,"tab_id":environment["tabs"][0]["tab_id"]
        }});
        let (result, read) = tokio::join!(
            dispatch_json(&fixture.home, request.clone()),
            dispatch_json(&fixture.home, read),
        );
        release.await.unwrap();
        read.expect("viewer read should wait for the route and complete");
        if !matches!(outcome, WaitingInput::Complete) {
            if matches!(outcome, WaitingInput::Cancel) {
                assert!(matches!(
                    result,
                    Err(DaemonError::BrowserControllerActionCancelled {
                        controller_fenced: false
                    })
                ));
            } else {
                assert!(result
                    .unwrap_err()
                    .to_string()
                    .contains("viewport changed while waiting"));
            }
            assert!(
                !log.exists(),
                "cancelled or stale waiting input must never reach the worker helper"
            );
            let environment = fixture
                .home
                .runtime_state
                .room_environment_snapshot(room)
                .unwrap();
            let clicks: Vec<_> = environment
                .actions
                .iter()
                .filter(|action| action.kind == "pointer_click")
                .collect();
            assert_eq!(clicks.len(), 1);
            let expected = if matches!(outcome, WaitingInput::Cancel) {
                crate::session::EnvironmentActionState::Cancelled
            } else {
                crate::session::EnvironmentActionState::Failed
            };
            assert_eq!(clicks[0].state, expected);
            return;
        }
        let result = result.expect(
            "human click must wait for the background route, not fail with controller_failure",
        );
        let submitted = &result["RoomEnvironmentActionSubmitted"];
        let action_id = &submitted["action_id"];
        let actions = submitted["environment"]["actions"].as_array().unwrap();
        let matching: Vec<_> = actions
            .iter()
            .filter(|action| &action["action_id"] == action_id)
            .collect();
        assert_eq!(matching.len(), 1);
        assert_eq!(matching[0]["state"], "completed");
        let repeated = dispatch_json(&fixture.home, request).await.unwrap();
        assert_eq!(
            &repeated["RoomEnvironmentActionSubmitted"]["action_id"],
            action_id
        );
        assert_eq!(
            std::fs::read_to_string(&log).unwrap(),
            "pointer-click 320 180 left 1\n"
        );
        for (key, action) in [
            ("contended-key", json!({"kind":"keyboard_key","key":"ctrl+a","repeat":1})),
            ("contended-scroll", json!({"kind":"pointer_scroll","x":320,"y":180,"horizontal_steps":0,"vertical_steps":2})),
        ] {
            let route = slices.guard_environment_use("desktop", Some(room), "browser_controller.route").unwrap();
            let release = tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(100)).await;
                drop(route);
            });
            let request = json!({"SubmitRoomEnvironmentAction": {
                "session_id":room,"runtime_generation":environment["runtime_generation"],
                "viewport_revision":environment["viewport"]["revision"],
                "idempotency_key":key,"action":action
            }});
            let submitted = dispatch_json(&fixture.home, request.clone()).await;
            release.await.unwrap();
            let submitted = submitted.expect("keyboard and scroll input must also queue behind a route");
            let id = &submitted["RoomEnvironmentActionSubmitted"]["action_id"];
            let repeated = dispatch_json(&fixture.home, request).await.unwrap();
            assert_eq!(&repeated["RoomEnvironmentActionSubmitted"]["action_id"], id);
            let entries: Vec<_> = repeated["RoomEnvironmentActionSubmitted"]["environment"]["actions"].as_array().unwrap()
                .iter().filter(|entry| &entry["action_id"] == id).collect();
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0]["state"], "completed");
        }
        assert_eq!(std::fs::read_to_string(&log).unwrap(), concat!(
            "pointer-click 320 180 left 1\n", "computer-key-stdin 1\n", "pointer-scroll 320 180 0 2\n"));
    })
    .catch_unwind()
    .await;
    let cleanup = fixture
        .worker
        .runtime_state
        .shutdown_browser_controller_process()
        .await;
    fixture.stop().await;
    std::env::remove_var("CHARIOX_SLICE_SCREEN_TOOL");
    cleanup.unwrap();
    if let Err(panic) = assertions {
        std::panic::resume_unwind(panic);
    }
}
