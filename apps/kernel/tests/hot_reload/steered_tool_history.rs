use super::*;

const ORIGINAL_TOOL_ID: &str = "original-run";

async fn history(socket: &mut Socket, session: &str, agent: &str) -> Value {
    request(
        socket,
        json!({"GetSessionHistoryOutline": {"session_id": session, "agent_ids": [agent],
            "include_prefetched_content": true}}),
    )
    .await
}

fn original_tool_rows(history: &Value) -> Vec<Value> {
    history["SessionHistoryOutline"]["agents"][0]["prefetched_blobs"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|blob| blob["entries"].as_array().into_iter().flatten())
        .filter(|fragment| {
            let entry = &fragment["entry"];
            entry["kind"] == "provider_tool" && entry["merge_key"] == ORIGINAL_TOOL_ID
        })
        .map(|fragment| serde_json::from_str(fragment["entry"]["text"].as_str().unwrap()).unwrap())
        .collect()
}

#[tokio::test]
async fn steering_preserves_the_original_agents_run_result_in_history() {
    let scratch = std::env::temp_dir().join(format!("st-agg-{}", std::process::id()));
    let record = scratch.join("record");
    std::fs::create_dir_all(&record).unwrap();
    std::fs::write(record.join("steer-tool"), "").unwrap();
    let claude = scratch.join("claude");
    install(
        |staged| std::fs::write(staged, FAKE_CLAUDE).unwrap(),
        &claude,
    );
    let config = scratch.join("claude-config");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(
        config.join(".credentials.json"),
        r#"{"claudeAiOauth":{"accessToken":"fixture","refreshToken":"fixture"}}"#,
    )
    .unwrap();
    let kernel = start_laptop_kernel(
        &scratch,
        &[
            ("CHARIOX_CLAUDE_BIN", claude),
            ("CHARIOX_FAKE_CLAUDE_RECORD", record),
            ("CLAUDE_CONFIG_DIR", config),
        ],
    );
    let mut socket = connect(&kernel, Duration::from_secs(60)).await;
    let project = scratch.join("project").display().to_string();
    let created = request(
        &mut socket,
        json!({"CreateSession": {"workspace_id": project, "worktree_id": project,
            "alias": null, "slice_ref": null, "no_subscribe": true}}),
    )
    .await;
    let session = created["SessionCreated"]["session"]["id"].as_str().unwrap();
    let spawned = request(
        &mut socket,
        json!({"SpawnAgent": {"session_id": session, "alias": "parent", "provider": "claude",
            "model": "claude-haiku-5-5", "effort": null, "worktree_id": null, "no_subscribe": true}}),
    )
    .await;
    let agent = spawned["AgentSpawned"]["agent"]["id"].as_str().unwrap();
    let attached = request(
        &mut socket,
        json!({"AttachToSession": {"session_id": session,
            "client_id": "steered-tool-history", "capability_level": "FullTerminal"}}),
    )
    .await;
    let attachment = attached["SessionAttached"]["attachment"]["id"]
        .as_str()
        .unwrap();
    request(
        &mut socket,
        json!({"SubmitPrompt": {"session_id": session, "attachment_id": attachment,
            "target_agent_id": agent, "prompt": "Run a worker", "attachments": []}}),
    )
    .await;
    let deadline = Instant::now() + Duration::from_secs(30);
    let prompt = loop {
        let before = history(&mut socket, session, agent).await;
        if original_tool_rows(&before)
            .iter()
            .any(|row| row["status"] == "running")
        {
            let turn = &before["SessionHistoryOutline"]["agents"][0]["turns"][0];
            break turn["prompt_id"].as_str().unwrap().to_string();
        }
        assert!(
            Instant::now() < deadline,
            "The original run never appeared in history"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let steered = request(
        &mut socket,
        json!({"SteerActivePrompt": {"session_id": session, "attachment_id": attachment,
            "target_agent_id": agent, "expected_active_prompt_id": prompt,
            "prompt": "Include STEER-MARKER", "attachments": []}}),
    )
    .await;
    assert!(steered.get("ActivePromptSteered").is_some(), "{steered}");

    let deadline = Instant::now() + Duration::from_secs(30);
    let after = loop {
        let after = history(&mut socket, session, agent).await;
        let turn = &after["SessionHistoryOutline"]["agents"][0]["turns"][0];
        if turn["lifecycle"] == "completed" {
            assert_eq!(turn["prompt_id"], prompt);
            break after;
        }
        assert!(
            Instant::now() < deadline,
            "The original parent turn did not complete"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let rows = original_tool_rows(&after);
    let last = rows
        .last()
        .expect("The original tool is missing from history");
    assert_eq!(
        last["status"], "completed",
        "Original agents.run history: {rows:?}"
    );
    assert!(last["output"]
        .as_str()
        .unwrap_or_default()
        .contains("WORKER-DONE"));
}
