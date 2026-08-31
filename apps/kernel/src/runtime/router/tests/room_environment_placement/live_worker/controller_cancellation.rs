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
        assert_eq!(
            cancelled["RoomEnvironmentActionCancellationUpdated"]["outcome"]["state"],
            "cancellation_requested"
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
