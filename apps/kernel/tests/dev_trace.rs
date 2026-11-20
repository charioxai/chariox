//! Drive generated traces through the same kernel socket and prompt path as a client.
#![cfg(unix)]

use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

#[path = "support/real_kernel.rs"]
mod real_kernel;
use real_kernel::{connect, create_session, request, Socket};

const FIXTURE: &str = include_str!("../../../scripts/drills/loadlab/fixtures/orch.jsonl");

async fn spawn(socket: &mut Socket, room: &str, alias: &str) -> String {
    let response = request(
        socket,
        json!({"SpawnAgent": {"session_id": room, "alias": alias,
            "provider": "dev-stub", "model": "trace", "no_subscribe": true}}),
    )
    .await
    .unwrap();
    response["AgentSpawned"]["agent"]["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

async fn submit(socket: &mut Socket, room: &str, agent: &str) -> Result<Value, Value> {
    let response = request(
        socket,
        json!({"AttachToSession": {"session_id": room, "client_id": "loadlab",
            "capability_level": "FullTerminal"}}),
    )
    .await
    .unwrap();
    let attachment = response["SessionAttached"]["attachment"]["id"]
        .as_str()
        .unwrap();
    request(
        socket,
        json!({"SubmitPrompt": {"session_id": room, "attachment_id": attachment,
            "target_agent_id": agent, "prompt": "Generated lab replay.", "attachments": []}}),
    )
    .await
}

fn start(scratch: &Path) -> real_kernel::Kernel {
    real_kernel::start(
        scratch,
        &[
            ("CHARIOX_PROVIDER_DEV_STUB", "1".into()),
            ("CHARIOX_LOADLAB_TRACE_DIR", scratch.as_os_str().to_owned()),
        ],
        None,
    )
}

async fn completed_turn(socket: &mut Socket, room: &str, agent: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let response = request(
            socket,
            json!({"GetSessionHistoryOutline": {"session_id": room, "agent_ids": [agent],
                "include_prefetched_content": true}}),
        )
        .await
        .unwrap();
        let turn = &response["SessionHistoryOutline"]["agents"][0]["turns"][0];
        if let Some("completed" | "cancelled" | "failed") = turn["lifecycle"].as_str() {
            return turn.clone();
        }
        assert!(Instant::now() < deadline, "turn did not settle: {response}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn trace_projects_structured_history_and_outcomes() {
    let scratch = std::env::temp_dir().join(format!("loadlab-rows-{}", std::process::id()));
    let kernel = start(&scratch);
    std::fs::write(scratch.join("orch.jsonl"), FIXTURE).unwrap();
    for (alias, outcome) in [("cancelled", "cancelled"), ("failed", "error")] {
        let fixture = FIXTURE
            .replace(r#""agent":"orch""#, &format!(r#""agent":"{alias}""#))
            .replace(
                r#""outcome":"completed""#,
                &format!(r#""outcome":"{outcome}""#),
            );
        std::fs::write(scratch.join(format!("{alias}.jsonl")), fixture).unwrap();
    }
    let mut socket = connect(&kernel).await;
    let room = create_session(&mut socket, &scratch).await;
    for (alias, lifecycle) in [
        ("orch", "completed"),
        ("cancelled", "cancelled"),
        ("failed", "failed"),
    ] {
        let agent = spawn(&mut socket, &room, alias).await;
        submit(&mut socket, &room, &agent).await.unwrap();
        let turn = completed_turn(&mut socket, &room, &agent).await;
        assert_eq!(turn["lifecycle"], lifecycle, "{turn}");
        assert!(turn["outcome"]["kind"].is_string(), "{turn}");
        let kinds = turn["blobs"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|blob| blob["kind"].as_str())
            .collect::<Vec<_>>();
        for kind in ["provider_reasoning", "provider_output", "provider_tool"] {
            assert!(kinds.contains(&kind), "expected {kind} in {turn}");
        }
    }
}

#[tokio::test]
async fn trace_refuses_to_launch_without_opt_in() {
    let scratch = std::env::temp_dir().join(format!("loadlab-gate-{}", std::process::id()));
    let kernel = real_kernel::start(&scratch, &[], None);
    let mut socket = connect(&kernel).await;
    let room = create_session(&mut socket, &scratch).await;
    let error = request(
        &mut socket,
        json!({"LaunchProviderRun": {"session_id": room, "adapter_key": "dev-stub",
          "provider": "dev-stub", "account_profile": "default", "model": "trace"}}),
    )
    .await
    .unwrap_err();
    assert_eq!(error["code"], "provider_dev_stub_disabled");
}
