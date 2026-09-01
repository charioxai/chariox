use super::*;

pub(super) async fn check(fixture: &LiveWorker, token: &str) {
    let before = fixture
        .home
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(token, "slice_browser_status", json!({}))
        .await
        .expect("capture the Room browser before crashing its worker controller");
    let before_environment = dispatch_json(
        &fixture.home,
        json!({"GetRoomEnvironmentState":{"session_id":fixture.rooms[0]}}),
    )
    .await
    .unwrap();
    let before_environment = &before_environment["RoomEnvironmentState"]["environment"];
    let before_action_count = before_environment["actions"].as_array().unwrap().len();
    let before_ownership = before_environment["input_ownership"].clone();
    let old_field = before.payload["browser"]["buttons"][0]["field_id"]
        .as_str()
        .unwrap()
        .to_string();
    let old_pid = std::fs::read_to_string(fixture._worker_state.root.join("controller.pid"))
        .unwrap()
        .trim()
        .parse::<u32>()
        .unwrap();
    assert!(crate::runtime::process_health::process_running(old_pid));

    #[cfg(unix)]
    {
        let killed = unsafe { libc::kill(old_pid as libc::pid_t, libc::SIGKILL) };
        assert_eq!(killed, 0, "kill the fixture-owned controller process");
    }
    #[cfg(not(unix))]
    compile_error!("Room controller crash recovery drill requires Unix signals");

    tokio::time::sleep(Duration::from_millis(50)).await;
    let stale_error = fixture
        .home
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            token,
            "slice_browser_click",
            json!({"field_id":old_field}),
        )
        .await
        .expect_err("a mutation must not run across an implicit controller restart");
    assert!(
        stale_error
            .to_string()
            .contains("browser controller restarted before the operation"),
        "{stale_error}"
    );
    let repeated_stale_error = fixture
        .home
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            token,
            "slice_browser_click",
            json!({"field_id":old_field}),
        )
        .await
        .expect_err("the restarted controller must invalidate the old element reference");
    assert!(
        repeated_stale_error.to_string().contains("stale"),
        "{repeated_stale_error}"
    );
    let after_failed_mutation = dispatch_json(
        &fixture.home,
        json!({"GetRoomEnvironmentState":{"session_id":fixture.rooms[0]}}),
    )
    .await
    .unwrap();
    let after_failed_mutation = &after_failed_mutation["RoomEnvironmentState"]["environment"];
    assert_eq!(
        after_failed_mutation["actions"].as_array().unwrap().len(),
        before_action_count + 1,
        "a rejected stale mutation must have one attributed terminal action"
    );
    assert_eq!(
        after_failed_mutation["actions"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["state"],
        "failed"
    );
    assert_eq!(
        after_failed_mutation["actions"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["outcome"]["code"],
        "process_lost"
    );

    let recovered = fixture
        .home
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(token, "slice_browser_status", json!({}))
        .await
        .expect("the next public observation recovers and reconciles the Room browser");
    let recovered_pid = std::fs::read_to_string(fixture._worker_state.root.join("controller.pid"))
        .unwrap()
        .trim()
        .parse::<u32>()
        .unwrap();
    assert_ne!(recovered_pid, old_pid);
    assert!(crate::runtime::process_health::process_running(
        recovered_pid
    ));
    assert_eq!(
        recovered.payload["environment_id"],
        before.payload["environment_id"]
    );
    assert_eq!(
        recovered.payload["runtime_generation"],
        before.payload["runtime_generation"]
    );
    assert_eq!(
        recovered.payload["tabs"], before.payload["tabs"],
        "controller recovery must reconcile the existing Room tabs without duplication"
    );

    let recovered_environment = dispatch_json(
        &fixture.home,
        json!({"GetRoomEnvironmentState":{"session_id":fixture.rooms[0]}}),
    )
    .await
    .unwrap();
    let recovered_environment = &recovered_environment["RoomEnvironmentState"]["environment"];
    for component in ["browser_controller", "browser"] {
        assert!(
            recovered_environment["health"]
                .as_array()
                .unwrap()
                .iter()
                .any(|health| health["component"] == component && health["state"] == "ready"),
            "{component} must be ready after controller crash recovery"
        );
    }
    let ownership = recovered_environment["input_ownership"].as_array().unwrap();
    assert_eq!(
        recovered_environment["input_ownership"], before_ownership,
        "controller recovery must preserve existing input authority"
    );
    let unique_targets = ownership
        .iter()
        .map(|owner| owner["target"].to_string())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        ownership.len(),
        unique_targets.len(),
        "controller recovery must not duplicate input authority"
    );

    let fresh_field = recovered.payload["browser"]["buttons"][0]["field_id"]
        .as_str()
        .unwrap();
    let completed = fixture
        .home
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            token,
            "slice_browser_click",
            json!({"field_id":fresh_field}),
        )
        .await
        .expect("a fresh post-recovery mutation executes");
    assert!(completed.ok, "{:?}", completed.payload);
    let after_completed = dispatch_json(
        &fixture.home,
        json!({"GetRoomEnvironmentState":{"session_id":fixture.rooms[0]}}),
    )
    .await
    .unwrap();
    let after_completed = &after_completed["RoomEnvironmentState"]["environment"];
    assert_eq!(
        after_completed["actions"].as_array().unwrap().len(),
        before_action_count + 2,
        "one fresh mutation must create exactly one additional action"
    );
    assert_eq!(
        after_completed["actions"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["state"],
        "completed"
    );

    let dialog_crash_pid =
        std::fs::read_to_string(fixture._worker_state.root.join("controller.pid"))
            .unwrap()
            .trim()
            .parse::<u32>()
            .unwrap();
    assert_eq!(
        dialog_crash_pid, recovered_pid,
        "the successful fresh action must not silently replace its controller"
    );
    assert!(crate::runtime::process_health::process_running(
        dialog_crash_pid
    ));
    #[cfg(unix)]
    {
        let killed = unsafe { libc::kill(dialog_crash_pid as libc::pid_t, libc::SIGKILL) };
        assert_eq!(killed, 0, "crash the recovered fixture controller");
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    let dialog = fixture
        .home
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            token,
            "slice_browser_dialog",
            json!({"action":"dismiss"}),
        )
        .await
        .expect("dialog preflight recovers before admitting one fresh mutation");
    assert!(dialog.ok, "{:?}", dialog.payload);
    let after_dialog = dispatch_json(
        &fixture.home,
        json!({"GetRoomEnvironmentState":{"session_id":fixture.rooms[0]}}),
    )
    .await
    .unwrap();
    assert_eq!(
        after_dialog["RoomEnvironmentState"]["environment"]["actions"]
            .as_array()
            .unwrap()
            .len(),
        after_completed["actions"].as_array().unwrap().len() + 1,
        "dialog recovery must admit exactly one fresh action"
    );
    let dialog_action = after_dialog["RoomEnvironmentState"]["environment"]["actions"]
        .as_array()
        .unwrap()
        .last()
        .unwrap();
    assert_eq!(dialog_action["action_id"], dialog.payload["action_id"]);
    assert_eq!(dialog_action["kind"], "dialog");
    assert_eq!(dialog_action["state"], "completed");
    let after_dialog_pid =
        std::fs::read_to_string(fixture._worker_state.root.join("controller.pid"))
            .unwrap()
            .trim()
            .parse::<u32>()
            .unwrap();
    assert_ne!(after_dialog_pid, dialog_crash_pid);
    assert!(crate::runtime::process_health::process_running(
        after_dialog_pid
    ));

    let after_dialog_recovery = fixture
        .home
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(token, "slice_browser_status", json!({}))
        .await
        .expect("the Room remains usable after dialog-triggered recovery");
    assert_eq!(
        after_dialog_recovery.payload["tabs"],
        recovered.payload["tabs"]
    );
}
