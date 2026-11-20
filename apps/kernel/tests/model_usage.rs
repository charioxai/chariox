//! Play native App Server usage frames through a real kernel and its client API.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

#[path = "support/real_kernel.rs"]
mod real_kernel;
use real_kernel::{connect, create_session, request, Socket};

struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn codex_turn_model_usage_survives_kernel_restart() {
    let scratch = Scratch(std::env::temp_dir().join(format!("model-usage-{}", std::process::id())));
    std::fs::create_dir(&scratch.0).unwrap();
    std::fs::create_dir(scratch.0.join("project")).unwrap();
    let record = scratch.0.join("record");
    std::fs::create_dir(&record).unwrap();
    std::fs::create_dir(scratch.0.join("codex-home")).unwrap();
    let codex = scratch.0.join("codex");
    std::fs::write(
        &codex,
        format!(
            "#!/usr/bin/python3\n{}",
            include_str!("hot_reload/fake_codex.py")
        ),
    )
    .unwrap();
    std::fs::set_permissions(&codex, std::fs::Permissions::from_mode(0o755)).unwrap();

    let frame = |thread, turn, total: [u64; 3], last: [u64; 3]| {
        let breakdown = |[input, output, cached]: [u64; 3]| {
            json!({
                "inputTokens": input, "outputTokens": output, "cachedInputTokens": cached,
                "reasoningOutputTokens": 0, "totalTokens": input + output
            })
        };
        json!({
            "threadId": thread, "turnId": turn,
            "tokenUsage": {
                "total": breakdown(total), "last": breakdown(last),
                "modelContextWindow": 128000
            }
        })
    };
    let first = frame(
        "thread-1",
        "turn-1",
        [40000, 2100, 12000],
        [7600, 1300, 2000],
    );
    let second = frame("thread-1", "turn-1", [41000, 2300, 12700], [1000, 200, 700]);
    std::fs::write(
        record.join("usage.json"),
        json!({
            "model": "gpt-6.2-sol",
            "updates": [
                frame("thread-1", "old-turn", [32400, 800, 10000], [1000, 200, 700]),
                first,
                first,
                second,
                frame("another-thread", "turn-1", [99000, 6000, 10000], [99000, 6000, 10000])
            ]
        })
        .to_string(),
    )
    .unwrap();

    let env = [
        ("CHARIOX_HOME", scratch.0.join("state").into()),
        ("CHARIOX_DAEMON_ID", "recorded-model-usage-drill".into()),
        ("CHARIOX_CODEX_BIN", codex.into()),
        ("CHARIOX_FAKE_CODEX_RECORD", record.into()),
        ("CODEX_HOME", scratch.0.join("codex-home").into()),
    ];
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let expected = json!([{
        "provider": "codex", "model": "gpt-6.2-sol",
        "input_tokens": 8600, "output_tokens": 1500, "cached_input_tokens": 2700,
        "cache_write_input_tokens": 0, "api_equivalent_nanodollars": "27070000"
    }]);
    let first_kernel = real_kernel::start(&scratch.0.join("first"), &env, None);
    let (session, agent) = runtime.block_on(async {
        let mut socket = connect(&first_kernel).await;
        let room = create_session(&mut socket, &scratch.0).await;
        let agent = start_turn(&mut socket, &room).await;
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let response = request(
                &mut socket,
                json!({"GetSessionState": {"session_id": room}}),
            )
            .await
            .unwrap();
            if std::fs::read_to_string(scratch.0.join("record/parts"))
                .is_ok_and(|parts| parts.contains("result 1"))
                && response["SessionState"]["session"]["prompt_states"][&agent]["active_prompt"]
                    .is_null()
            {
                break;
            }
            assert!(Instant::now() < deadline, "Codex turn did not settle");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert_eq!(usage(&mut socket, &room, &agent).await, expected);
        (room, agent)
    });
    drop(first_kernel);
    let second_kernel = real_kernel::start(&scratch.0.join("second"), &env, None);
    runtime.block_on(async {
        let mut socket = connect(&second_kernel).await;
        assert_eq!(usage(&mut socket, &session, &agent).await, expected);
    });
}

async fn start_turn(socket: &mut Socket, room: &str) -> String {
    let spawned = request(
        socket,
        json!({"SpawnAgent": {"session_id": room, "alias": "writer",
            "provider": "codex", "model": "gpt-6.2-sol", "effort": "low",
            "worktree_id": null, "no_subscribe": true}}),
    )
    .await
    .unwrap();
    let agent = spawned["AgentSpawned"]["agent"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let attached = request(
        socket,
        json!({"AttachToSession": {"session_id": room,
            "client_id": "model-usage-test", "capability_level": "FullTerminal"}}),
    )
    .await
    .unwrap();
    let attachment = attached["SessionAttached"]["attachment"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    request(
        socket,
        json!({"SubmitPrompt": {"session_id": room, "attachment_id": attachment,
            "target_agent_id": agent, "prompt": "Reply with one word",
            "attachments": []}}),
    )
    .await
    .unwrap();
    agent
}

async fn usage(socket: &mut Socket, room: &str, agent: &str) -> Value {
    let response = request(
        socket,
        json!({"GetAgentUsageSnapshot": {"session_id": room, "agent_id": agent}}),
    )
    .await
    .unwrap();
    response["AgentUsageSnapshot"]["snapshot"]["models"].clone()
}
