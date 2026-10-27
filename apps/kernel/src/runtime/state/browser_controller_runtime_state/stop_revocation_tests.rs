use super::*;
use crate::runtime::state::browser_controller_action_execution_runtime_state::computer_input_reconcile_test_support::{TestRoom, TestTools};
use std::time::Duration;

#[tokio::test]
async fn dispatched_room_stop_settles_after_grant_revocation() {
    crate::test_support::isolated_env_test!();
    let tools = TestTools::new("revoked-room-release");
    let started = tools.controller_tool.with_file_name("release-started");
    let finish = tools.controller_tool.with_file_name("release-finish");
    let script = std::fs::read_to_string(&tools.controller_tool).unwrap()
            .replace("const { createInterface }", "const fs = require('node:fs');\nconst { createInterface }")
            .replace("} else if (method === \"shutdown\") {", &format!(r#"}} else if (method === "shutdown") {{
                fs.writeFileSync({}, 'waiting');
                while (!fs.existsSync({})) Atomics.wait(new Int32Array(new SharedArrayBuffer(4)),0,0,10);
"#,
                serde_json::to_string(&started).unwrap(), serde_json::to_string(&finish).unwrap()));
    std::fs::write(&tools.controller_tool, script).unwrap();
    let mut room = TestRoom::new("revoked-room-release");
    room.enable_browser_controller(&tools).await;
    let before = room
        .runtime
        .room_environment_snapshot(&room.session_id)
        .unwrap();
    let grant = room.runtime.insert_access_grant_for_test(&room.session_id);
    let request = crate::local::LocalDaemonRequest::StopRoomEnvironment(
        crate::local::StopRoomEnvironmentRequest {
            session_id: room.session_id.clone(),
        },
    );
    let external = room
        .runtime
        .with_external_command_authority(Some((&grant, &request)));
    let revoke = async {
        tokio::time::timeout(Duration::from_secs(5), async {
            while !started.exists() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            room.runtime
                .room_environment_snapshot(&room.session_id)
                .unwrap()
                .lifecycle,
            EnvironmentLifecycle::Stopping
        );
        room.runtime
            .revoke_kernel_access(None, Some(&grant), "explicit_revoke")
            .unwrap();
        std::fs::write(&finish, "finish").unwrap();
    };
    let (result, ()) = tokio::join!(
        external.stop_managed_room_environment_runtime(&room.session_id),
        revoke
    );
    let settled = room
        .runtime
        .room_environment_snapshot(&room.session_id)
        .unwrap();
    let generation_cleared = !room
        .runtime
        .owned
        .browser_controller_generations
        .lock()
        .unwrap()
        .contains_key(&room.session_id);
    assert_eq!(
        settled.lifecycle,
        EnvironmentLifecycle::Stopped,
        "started stop must settle despite revoked grant: {result:?}"
    );
    assert!(result.is_ok(), "completed release must retain its result");
    assert!(
        generation_cleared,
        "released controller generation must be cleared"
    );
    let started = room
        .runtime
        .start_room_environment(&room.session_id, before.viewport.clone())
        .expect("trusted Start must admit the stopped Room");
    let restarted = room
        .runtime
        .finish_room_environment_controller_start(&room.session_id, "test.start")
        .await;
    assert!(started.runtime_generation > before.runtime_generation);
    room.stop_browser_controller().await;
    let restarted = restarted.expect("trusted Start after settled Stop must succeed");
    assert!(restarted.runtime_generation > before.runtime_generation);
    assert_ne!(restarted.lifecycle, EnvironmentLifecycle::Stopping);
}
