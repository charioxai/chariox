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
    let controller_state_path = fixture._worker_state.root.join("chromium-state.json");
    let clicks_before_recovery = controller_click_count(&controller_state_path);
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
    assert_eq!(
        controller_click_count(&controller_state_path),
        clicks_before_recovery,
        "a stale-reference failure must not dispatch physical input"
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
    let failed_action_count = after_failed_mutation["actions"].as_array().unwrap().len();
    let recovered_actions = recovered_environment["actions"].as_array().unwrap();
    assert_eq!(
        recovered_actions.len(),
        failed_action_count + 1,
        "the post-restart status read records one browser observation"
    );
    let recovery_observation = recovered_actions.last().unwrap();
    assert_eq!(recovery_observation["kind"], "browser_status");
    assert_eq!(recovery_observation["state"], "completed");
    assert_eq!(
        recovery_observation["actor_id"],
        crate::session::agent_environment_actor_id(
            before.payload["agent_id"].as_str().unwrap()
        ),
        "the recovery observation remains attributed to the requesting agent"
    );
    assert_eq!(
        recovery_observation["runtime_generation"],
        before.payload["runtime_generation"]
    );
    assert_eq!(
        recovery_observation["targets"][0],
        json!({"kind":"browser_tab","id":before.payload["tab_id"]})
    );
    assert_eq!(
        controller_click_count(&controller_state_path),
        clicks_before_recovery,
        "recovering through a browser status observation must not replay a click"
    );
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
        recovered_actions.len() + 1,
        "one fresh mutation must append one action after the recovery observation"
    );
    let completed_action = after_completed["actions"].as_array().unwrap().last().unwrap();
    assert_eq!(completed_action["action_id"], completed.payload["action_id"]);
    assert_eq!(completed_action["kind"], "click");
    assert_eq!(
        completed_action["state"],
        "completed"
    );
    assert_eq!(
        completed_action["actor_id"],
        crate::session::agent_environment_actor_id(
            before.payload["agent_id"].as_str().unwrap()
        )
    );
    assert_eq!(
        completed_action["runtime_generation"],
        recovered.payload["runtime_generation"]
    );
    assert_eq!(
        completed_action["targets"][0],
        json!({"kind":"browser_tab","id":recovered.payload["tab_id"]})
    );
    assert_eq!(
        controller_click_count(&controller_state_path),
        clicks_before_recovery + 1,
        "the fresh post-recovery click is physically dispatched exactly once"
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

    let queued_field = after_dialog_recovery.payload["browser"]["buttons"][0]["field_id"]
        .as_str()
        .expect("recovered browser button reference")
        .to_string();
    let clicks_before_queue_fault = controller_click_count(&controller_state_path);
    let actions_before_queue_fault = dispatch_json(
        &fixture.home,
        json!({"GetRoomEnvironmentState":{"session_id":fixture.rooms[0]}}),
    )
    .await
    .unwrap()["RoomEnvironmentState"]["environment"]["actions"]
        .as_array()
        .unwrap()
        .len();
    let hold = fixture._worker_state.root.join("hold-click");
    std::fs::write(&hold, b"hold controller mutation during crash").unwrap();
    let running = controller_fault_tool_task(
        fixture,
        token,
        "slice_browser_click",
        json!({"field_id":queued_field}),
    );
    let running_action_id =
        wait_for_fault_action(fixture, actions_before_queue_fault, "running").await;
    let queued = controller_fault_tool_task(
        fixture,
        token,
        "slice_browser_click",
        json!({"field_id":queued_field}),
    );
    let queued_action_id =
        wait_for_fault_action(fixture, actions_before_queue_fault + 1, "queued").await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let queue_fault_pid =
        std::fs::read_to_string(fixture._worker_state.root.join("controller.pid"))
            .unwrap()
            .trim()
            .parse::<u32>()
            .unwrap();
    #[cfg(unix)]
    {
        let killed = unsafe { libc::kill(queue_fault_pid as libc::pid_t, libc::SIGKILL) };
        assert_eq!(
            killed, 0,
            "crash the controller while one mutation runs and another waits"
        );
    }
    let _ = std::fs::remove_file(&hold);
    let running_error = tokio::time::timeout(Duration::from_secs(6), running)
        .await
        .expect("running mutation should settle after controller loss")
        .expect("running mutation task should join")
        .expect_err("running mutation must not report success after controller loss");
    let queued_error = tokio::time::timeout(Duration::from_secs(6), queued)
        .await
        .expect("queued mutation should settle after controller loss")
        .expect("queued mutation task should join")
        .expect_err("queued pre-crash mutation must not execute with a stale reference");
    assert_controller_execution_loss(&running_error);
    assert_controller_restart_error(&queued_error);

    let after_queue_fault = dispatch_json(
        &fixture.home,
        json!({"GetRoomEnvironmentState":{"session_id":fixture.rooms[0]}}),
    )
    .await
    .unwrap();
    let queue_fault_actions = after_queue_fault["RoomEnvironmentState"]["environment"]["actions"]
        .as_array()
        .unwrap();
    for action_id in [&running_action_id, &queued_action_id] {
        let action = queue_fault_actions
            .iter()
            .find(|action| action["action_id"] == *action_id)
            .expect("pre-crash action remains in the authoritative ledger");
        assert_eq!(
            action["state"], "failed",
            "pre-crash mutation must settle failed: {action}"
        );
    }
    assert_eq!(
        controller_click_count(&controller_state_path),
        clicks_before_queue_fault,
        "controller recovery must not repeat a running or queued mutation"
    );

    let after_queue_recovery = fixture
        .home
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(token, "slice_browser_status", json!({}))
        .await
        .expect("Room browser should recover after a queued-mutation crash");
    let post_recovery_field = after_queue_recovery.payload["browser"]["buttons"][0]["field_id"]
        .as_str()
        .expect("post-recovery browser button reference");
    fixture
        .home
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            token,
            "slice_browser_click",
            json!({"field_id":post_recovery_field}),
        )
        .await
        .expect("one newly discovered post-recovery mutation should execute");
    assert_eq!(
        controller_click_count(&controller_state_path),
        clicks_before_queue_fault + 1,
        "only the explicit post-recovery mutation may change the page"
    );
    super::controller_upload_recovery::check_restart(fixture, token).await;
    super::controller_configuration_recovery::check(fixture, token).await;
    super::controller_lifecycle_cancellation::check(fixture, token).await;
    super::controller_navigation_queue::check(fixture, token).await;
    eprintln!(
        "{}",
        json!({
            "schema": "chariox.browser_controller_fault_probe.v2",
            "faultTriggered": true,
            "processLostAttributed": true,
            "staleReferenceRejected": true,
            "processReplaced": true,
            "tabsPreserved": true,
            "authorityPreserved": true,
            "postRecoveryActionExactlyOnce": true,
            "runningMutationNotRepeated": true,
            "queuedMutationSettled": true,
            "freshMutationExactlyOnce": true
        })
    );
}

fn controller_fault_tool_task(
    fixture: &LiveWorker,
    token: &str,
    tool: &'static str,
    args: Value,
) -> JoinHandle<Result<Value, DaemonError>> {
    let home = Arc::clone(&fixture.home);
    let token = token.to_string();
    tokio::spawn(async move {
        let result = home
            .runtime_state
            .dispatch_authenticated_runtime_tool_call(&token, tool, args)
            .await?;
        assert!(result.ok, "{:?}", result.payload);
        Ok(result.payload)
    })
}

async fn wait_for_fault_action(
    fixture: &LiveWorker,
    minimum_prior_actions: usize,
    state: &str,
) -> Value {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let snapshot = dispatch_json(
                &fixture.home,
                json!({"GetRoomEnvironmentState":{"session_id":fixture.rooms[0]}}),
            )
            .await
            .unwrap();
            let actions = snapshot["RoomEnvironmentState"]["environment"]["actions"]
                .as_array()
                .unwrap();
            if actions.len() > minimum_prior_actions {
                if let Some(action) = actions[minimum_prior_actions..]
                    .iter()
                    .find(|action| action["kind"] == "click" && action["state"] == state)
                {
                    return action["action_id"].clone();
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("queued-mutation fault action should become visible")
}

fn controller_click_count(path: &std::path::Path) -> u64 {
    serde_json::from_slice::<Value>(&std::fs::read(path).expect("controller state should exist"))
        .expect("controller state should decode")["clickCount"]
        .as_u64()
        .expect("controller click count")
}

fn assert_controller_restart_error(error: &DaemonError) {
    let DaemonError::LocalTransport { operation, message } = error else {
        panic!("controller loss should return a transport error: {error}")
    };
    assert_eq!(*operation, "browser_controller.route");
    assert_eq!(
        message,
        crate::runtime::browser_controller_process::CONTROLLER_RESTARTED_BEFORE_OPERATION
    );
}

fn assert_controller_execution_loss(error: &DaemonError) {
    let DaemonError::LocalTransport { operation, message } = error else {
        panic!("running controller loss should return a transport error: {error}")
    };
    assert_eq!(*operation, "browser_controller.route");
    assert!(
        message.contains(
            "browser action result remained unavailable after non-mutating receipt recovery"
        ) && message.contains("browser controller exited during `browser.action`"),
        "{error}"
    );
}
