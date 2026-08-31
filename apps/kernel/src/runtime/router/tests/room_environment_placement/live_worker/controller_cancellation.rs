use super::*;
use futures_util::FutureExt;

pub(super) async fn check(fixture: &LiveWorker, token: &str, status: &Value) {
    let hold = fixture._worker_state.root.join("hold-click");
    std::fs::write(&hold, b"disabled page button").unwrap();
    let first = tool_task(
        fixture,
        token,
        "slice_browser_click",
        json!({"field_id":status["browser"]["buttons"][0]["field_id"]}),
    );
    let mut second = None;
    let assertions = std::panic::AssertUnwindSafe(async {
        let running = wait_action(fixture, "click", "running").await;
        second = Some(tool_task(
            fixture,
            token,
            "slice_browser_submit",
            json!({"field_id":status["browser"]["fields"][0]["field_id"]}),
        ));
        let queued = wait_action(fixture, "submit", "queued").await;
        let takeover = dispatch_json(
            &fixture.home,
            json!({"RequestRoomEnvironmentInputTakeover":{
                "session_id":fixture.rooms[0],
                "target":{"kind":"browser_tab","id":status["tab_id"]}
            }}),
        )
        .await
        .expect("human can take over while browser input is running");
        let actions = takeover["RoomEnvironmentTakeoverUpdated"]["environment"]["actions"]
            .as_array()
            .unwrap();
        assert_eq!(
            actions
                .iter()
                .find(|action| action["action_id"] == queued)
                .unwrap()["state"],
            "cancelled"
        );
        let cancelled = dispatch_json(
            &fixture.home,
            json!({"CancelRoomEnvironmentAction":{
                "session_id":fixture.rooms[0],"action_id":running
            }}),
        )
        .await
        .expect("pending human owner can cancel ready-browser input while desktop starts");
        let outcome = &cancelled["RoomEnvironmentActionCancellationUpdated"]["outcome"];
        assert!(
            outcome["state"] == "cancellation_requested"
                || (outcome["state"] == "already_terminal"
                    && outcome["action_state"] == "cancelled"),
            "cancellation is accepted even if takeover already stopped input: {outcome}"
        );
    })
    .catch_unwind()
    .await;
    // Always release the external page and drain both public calls before the
    // enclosing fixture stops its worker, including on assertion failures.
    std::fs::remove_file(&hold).unwrap();
    let first_result = timeout(Duration::from_secs(6), first).await;
    let second_result = match second {
        Some(task) => Some(timeout(Duration::from_secs(6), task).await),
        None => None,
    };
    if let Err(panic) = assertions {
        std::panic::resume_unwind(panic);
    }
    // Physical interruption is asserted by its own drill, not by whether this
    // drain returns success or cancellation after the external page releases.
    let _ = first_result.expect("running browser call drains").unwrap();
    let queued_error = second_result
        .unwrap()
        .unwrap()
        .unwrap()
        .expect_err("cancelled queued input must not execute");
    assert!(
        queued_error.to_string().contains("Cancelled"),
        "{queued_error}"
    );
    let page = fixture
        .home
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(token, "slice_browser_status", json!({}))
        .await
        .unwrap();
    assert_eq!(
        page.payload["browser"]["buttons"][0]["label"], "Saved on worker",
        "queued submit must not change the page"
    );
    dispatch_json(
        &fixture.home,
        json!({"ReleaseRoomEnvironmentInput":{
            "session_id":fixture.rooms[0],
            "target":{"kind":"browser_tab","id":status["tab_id"]}
        }}),
    )
    .await
    .unwrap();
}

pub(super) async fn check_running(fixture: &LiveWorker, token: &str, status: &Value) {
    check_pending_cleanup(fixture, token, status, Duration::from_millis(100), false).await;
    // The fixture's worker driver has a three-second response timeout. Crossing
    // it must not turn uncertainty into permission to send human input.
    check_pending_cleanup(fixture, token, status, Duration::from_millis(3300), false).await;
    check_pending_cleanup(fixture, token, status, Duration::from_millis(8300), true).await;
}

async fn check_pending_cleanup(
    fixture: &LiveWorker,
    token: &str,
    status: &Value,
    delay: Duration,
    expect_fence: bool,
) {
    let hold = fixture._worker_state.root.join("hold-fill");
    let hold_release = fixture._worker_state.root.join("hold-release");
    let release_pending = fixture._worker_state.root.join("release-pending");
    if release_pending.exists() {
        std::fs::remove_file(&release_pending).unwrap();
    }
    std::fs::write(&hold, b"disabled page field").unwrap();
    std::fs::write(&hold_release, b"browser cleanup awaiting response").unwrap();
    let fill = tool_task(
        fixture,
        token,
        "slice_browser_fill",
        json!({"field_id":status["browser"]["fields"][0]["field_id"],"text":"must not be entered"}),
    );
    let controller_pid = std::fs::read_to_string(fixture._worker_state.root.join("controller.pid"))
        .unwrap()
        .trim()
        .parse::<u32>()
        .unwrap();
    let target = json!({"kind":"browser_tab","id":status["tab_id"]});
    let assertions = std::panic::AssertUnwindSafe(async {
        let running = wait_action(fixture, "fill", "running").await;
        // Wait for the browser's object release, not just ledger admission, so
        // the test holds a physical request across the cancellation handshake.
        timeout(Duration::from_secs(2), async {
            while !release_pending.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("external browser cleanup becomes pending");
        dispatch_json(
            &fixture.home,
            json!({"RequestRoomEnvironmentInputTakeover":{
                "session_id":fixture.rooms[0],"target":target
            }}),
        )
        .await
        .unwrap();
        tokio::time::sleep(delay).await;
        let pending = dispatch_json(
            &fixture.home,
            json!({"GetRoomEnvironmentState":{
                "session_id":fixture.rooms[0]
            }}),
        )
        .await
        .unwrap();
        let owners = pending["RoomEnvironmentState"]["environment"]["input_ownership"]
            .as_array()
            .unwrap();
        let human_owns = owners.iter().any(|owner| {
            owner["target"] == target && owner["actor_id"].as_str().unwrap().starts_with("user:")
        });
        if expect_fence {
            assert!(
                human_owns,
                "a completed physical fence must finalize takeover"
            );
            assert!(
                fill.is_finished(),
                "a completed physical fence must finish the call"
            );
            assert!(
                !crate::runtime::process_health::process_running(controller_pid),
                "human ownership requires the timed-out controller to be physically stopped"
            );
            let recovered_pid = timeout(Duration::from_secs(2), async {
                loop {
                    let pid =
                        std::fs::read_to_string(fixture._worker_state.root.join("controller.pid"))
                            .unwrap()
                            .trim()
                            .parse::<u32>()
                            .unwrap();
                    if pid != controller_pid && crate::runtime::process_health::process_running(pid)
                    {
                        break pid;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("a fenced controller must restart before cancellation completes");
            assert_ne!(recovered_pid, controller_pid);
            let recovered = dispatch_json(
                &fixture.home,
                json!({"GetRoomEnvironmentState":{
                    "session_id":fixture.rooms[0]
                }}),
            )
            .await
            .unwrap();
            let environment = &recovered["RoomEnvironmentState"]["environment"];
            assert_eq!(
                environment["tabs"], status["tabs"],
                "controller recovery must preserve stable Room tabs"
            );
            for component in ["browser_controller", "browser"] {
                assert!(
                    environment["health"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|health| {
                            health["component"] == component && health["state"] == "ready"
                        }),
                    "{component} must be ready after fenced recovery"
                );
            }
            assert_eq!(
                environment["input_ownership"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|owner| owner["target"] == target)
                    .count(),
                1,
                "controller recovery must not duplicate input authority"
            );
        } else {
            assert!(
                !human_owns,
                "human ownership must wait for physical completion"
            );
            assert!(
                !fill.is_finished(),
                "a cancel acknowledgement cannot finish pending physical cleanup"
            );
        }
        std::fs::remove_file(&hold_release).unwrap();
        timeout(Duration::from_secs(1), async {
            while !fill.is_finished() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("takeover must physically stop pending input before the page enables it");
        let snapshot = dispatch_json(
            &fixture.home,
            json!({"GetRoomEnvironmentState":{
                "session_id":fixture.rooms[0]
            }}),
        )
        .await
        .unwrap();
        let environment = &snapshot["RoomEnvironmentState"]["environment"];
        assert_eq!(
            environment["actions"]
                .as_array()
                .unwrap()
                .iter()
                .find(|action| action["action_id"] == running)
                .unwrap()["state"],
            "cancelled"
        );
        assert!(environment["input_ownership"]
            .as_array()
            .unwrap()
            .iter()
            .any(|owner| owner["target"] == target
                && owner["actor_id"].as_str().unwrap().starts_with("user:")));
    })
    .catch_unwind()
    .await;
    if hold_release.exists() {
        std::fs::remove_file(&hold_release).unwrap();
    }
    std::fs::remove_file(&hold).unwrap();
    let result = timeout(Duration::from_secs(6), fill).await;
    if let Err(panic) = assertions {
        std::panic::resume_unwind(panic);
    }
    let error = result
        .unwrap()
        .unwrap()
        .expect_err("cancelled input must not report success");
    assert!(
        error.to_string().to_lowercase().contains("cancel"),
        "{error}"
    );
    dispatch_json(
        &fixture.home,
        json!({"ReleaseRoomEnvironmentInput":{
            "session_id":fixture.rooms[0],"target":target
        }}),
    )
    .await
    .unwrap();
    let fresh = fixture
        .home
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(token, "slice_browser_status", json!({}))
        .await
        .unwrap();
    fixture
        .home
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            token,
            "slice_browser_submit",
            json!({"field_id":fresh.payload["browser"]["fields"][0]["field_id"]}),
        )
        .await
        .unwrap();
    let page = fixture
        .home
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(token, "slice_browser_status", json!({}))
        .await
        .unwrap();
    assert_eq!(
        page.payload["browser"]["buttons"][0]["label"], "Submitted: A note from home",
        "cancellation and controller recovery must preserve the external browser state"
    );
}

fn tool_task(
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

async fn wait_action(fixture: &LiveWorker, kind: &str, state: &str) -> Value {
    timeout(Duration::from_secs(2), async {
        loop {
            let snapshot = dispatch_json(
                &fixture.home,
                json!({"GetRoomEnvironmentState":{
                    "session_id":fixture.rooms[0]
                }}),
            )
            .await
            .unwrap();
            if let Some(action) = snapshot["RoomEnvironmentState"]["environment"]["actions"]
                .as_array()
                .unwrap()
                .iter()
                .find(|action| action["kind"] == kind && action["state"] == state)
            {
                return action["action_id"].clone();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("public action state becomes visible")
}
