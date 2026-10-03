use super::*;
use futures_util::FutureExt;

#[test]
fn ready_room_state_reads_do_not_put_reconciliation_ahead_of_pointer_input() {
    run_test(ready_state_reads_do_not_delay_pointer_input);
}

async fn ready_state_reads_do_not_delay_pointer_input() {
    let mut fixture = LiveWorker::start_configured(true, true).await;
    let root = &fixture._worker_state.root;
    let hold = root.join("hold-room-read-reconcile");
    let pending = root.join("room-read-reconcile-pending");
    let helper = root.join("input-helper.sh");
    let effects = root.join("input-effects.log");
    std::fs::write(
        &helper,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\n",
            effects.display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::env::set_var("CHARIOX_SLICE_SCREEN_TOOL", &helper);
    let assertions = std::panic::AssertUnwindSafe(async {
        fixture.create_slice().await;
        let room = &fixture.rooms[0];
        dispatch_json(&fixture.home, bind(room, "desktop")).await.unwrap();
        let started = dispatch_json(&fixture.home, json!({"StartRoomEnvironment": {
            "session_id":room,"viewport":{
                "css_width":1280,"css_height":800,"device_scale_factor":1,
                "desktop_pixel_width":1280,"desktop_pixel_height":800
            }
        }})).await.unwrap();
        let environment = &started["RoomEnvironmentUpdated"]["environment"];
        assert_eq!(environment["lifecycle"], "ready");
        dispatch_json(&fixture.home, json!({"RequestRoomEnvironmentInputTakeover":{
            "session_id":room,"target":{"kind":"desktop"}
        }})).await.unwrap();
        // A slow physical-display observation must never become a prerequisite
        // for input merely because a viewer read the already-Ready snapshot.
        std::fs::write(&hold, "hold").unwrap();
        for _ in 0..3 {
            let read = dispatch_json(&fixture.home, json!({"GetRoomEnvironmentState":{"session_id":room}})).await.unwrap();
            assert_eq!(read["RoomEnvironmentState"]["environment"]["lifecycle"], "ready");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(pending.exists(), "Ready reads must still observe browser tabs");
        let result = timeout(Duration::from_secs(1), dispatch_json(&fixture.home, json!({"SubmitRoomEnvironmentAction":{
            "session_id":room,"runtime_generation":environment["runtime_generation"],
            "viewport_revision":environment["viewport"]["revision"],
            "idempotency_key":"read-then-click",
            "action":{"kind":"pointer_click","x":320,"y":180,"button":"left","click_count":1}
        }}))).await;
        // Post-input reconciliation is permitted after the ledger has finished.
        // Read the ledger instead of requiring the submit ACK to precede it.
        let snapshot = fixture.home.runtime_state.room_environment_snapshot(room).unwrap();
        let action = snapshot.actions.iter().find(|action| action.idempotency_key.as_deref() == Some("read-then-click"));
        assert!(action.is_some_and(|action| action.state == crate::session::EnvironmentActionState::Completed), "Ready state reads blocked the input ledger: {action:?}; submit={result:?}");
        assert_eq!(std::fs::read_to_string(&effects).unwrap(), "pointer-click 320 180 left 1\n");
        std::fs::remove_file(&hold).unwrap();
        // Starting/Degraded reads retain controller recovery. A fresh probe
        // reconciles and returns the same Room to Ready after the gate clears.
        fixture.home.runtime_state.transition_room_environment(room, crate::session::EnvironmentLifecycle::Degraded).unwrap();
        dispatch_json(&fixture.home, json!({"GetRoomEnvironmentState":{"session_id":room}})).await.unwrap();
        timeout(Duration::from_secs(3), async {
            loop {
                dispatch_json(&fixture.home, json!({"GetRoomEnvironmentState":{"session_id":room}})).await.unwrap();
                if fixture.home.runtime_state.room_environment_snapshot(room).unwrap().lifecycle == crate::session::EnvironmentLifecycle::Ready { break; }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.expect("Degraded reads must still recover the controller");
    }).catch_unwind().await;
    let _ = std::fs::remove_file(&hold);
    let _ = std::fs::remove_file(&pending);
    std::env::remove_var("CHARIOX_SLICE_SCREEN_TOOL");
    fixture.stop().await;
    if let Err(panic) = assertions {
        std::panic::resume_unwind(panic);
    }
}
