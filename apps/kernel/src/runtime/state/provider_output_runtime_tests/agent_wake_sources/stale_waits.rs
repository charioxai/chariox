//! Row 470: an agent reads every wait of its other tasks without changing one, and a second watcher
//! of the same command is the first.
use super::*;

async fn auth(f: &Fixture) -> String {
    f.app
        .lock()
        .await
        .providers
        .get_run(&f.run)
        .unwrap()
        .runtime_mcp_auth_token()
        .unwrap()
        .to_string()
}

/// C5: the agent lists every wait of its other tasks, and reads one wake and its task's wait,
/// without cancelling anything.
#[tokio::test]
async fn c5_the_agent_reads_every_earlier_wait_and_one_wake_without_changing_them() {
    let f = fixture("stale-waits-read", true).await;
    let first = f.begin("earlier-prompt");
    f.arm(&first, "older").unwrap();
    f.wait_on(&first, "completion-older");
    assert_eq!(f.task(&first.task_id).state, ExecutionState::Waiting);
    let second = f.begin(&f.prompt_id);
    let token = auth(&f).await;

    let listed = call(
        &f,
        &token,
        "chariox.events.subscriptions",
        &second,
        serde_json::json!({}),
    )
    .await
    .unwrap()
    .payload;
    let waits = listed["waits"].as_array().unwrap();
    assert_eq!(waits.len(), 1, "{listed}");
    assert_eq!(waits[0]["task_id"], first.task_id.as_str());
    assert_eq!(
        waits[0]["registration_ids"],
        serde_json::json!(["completion-older"])
    );
    assert!(waits[0]["deadline_ms"].as_u64().is_some());

    let one = call(
        &f,
        &token,
        "chariox.events.wakes",
        &second,
        serde_json::json!({"wake_id": "older"}),
    )
    .await
    .unwrap()
    .payload;
    assert_eq!(one["wake"]["id"], "older", "{one}");
    assert_eq!(one["task"]["task_id"], first.task_id.as_str());
    assert_eq!(one["task"]["state"], "waiting");
    // Reading changed nothing: the wake still runs and the earlier task still waits.
    assert_eq!(f.wake_state("older"), "running");
    assert_eq!(f.task(&first.task_id).state, ExecutionState::Waiting);
    let unknown = call(
        &f,
        &token,
        "chariox.events.wakes",
        &second,
        serde_json::json!({"wake_id": "no-such-wake"}),
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(unknown.contains("wake unavailable"), "{unknown}");
}

/// C4: the same command under the same label is one watch.
#[tokio::test]
async fn c4_a_second_watcher_of_the_same_command_is_the_first() {
    let f = fixture("stale-waits-watch", true).await;
    let first = f.begin("earlier-prompt");
    let mut wake = f.wake(&first, "unit-watch");
    wake.kind = "process".into();
    wake.label = "farm run".into();
    wake.command = vec!["tail".into(), "-f".into(), "farm.log".into()];
    wake.detail = serde_json::json!({"cwd": null});
    wake.state = "running".into();
    f.store()
        .agent_lifecycle(Operation::CreateWake {
            task: first.task_id.clone(),
            prompt: first.prompt_id.clone(),
            wake,
        })
        .unwrap();
    let second = f.begin(&f.prompt_id);
    let same = serde_json::json!({"label":"farm run","argv":["tail","-f","farm.log"]});
    let found = f
        .runtime
        .process_already_watched(&second, &same)
        .unwrap()
        .expect("the same command is already watched");
    assert_eq!(found.id, "unit-watch");
    for other in [
        serde_json::json!({"label":"farm run","argv":["tail","-f","other.log"]}),
        serde_json::json!({"label":"another label","argv":["tail","-f","farm.log"]}),
        serde_json::json!({"label":"farm run","argv":["tail","-f","farm.log"],"cwd":"sub"}),
    ] {
        assert!(
            f.runtime
                .process_already_watched(&second, &other)
                .unwrap()
                .is_none(),
            "{other}"
        );
    }
}
