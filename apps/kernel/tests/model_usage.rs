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

#[test]
fn claude_cumulative_model_usage_counts_only_each_turn_across_resume() {
    let scratch =
        Scratch(std::env::temp_dir().join(format!("claude-model-usage-{}", std::process::id())));
    let record = scratch.0.join("record");
    let config = scratch.0.join("claude-config");
    std::fs::create_dir_all(scratch.0.join("project")).unwrap();
    std::fs::create_dir_all(&record).unwrap();
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(
        config.join(".credentials.json"),
        r#"{"claudeAiOauth":{"accessToken":"fixture","refreshToken":"fixture"}}"#,
    )
    .unwrap();
    let claude = scratch.0.join("claude");
    std::fs::write(&claude, include_str!("model_usage/fake_claude.py")).unwrap();
    std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o700)).unwrap();
    let env = [
        ("CHARIOX_HOME", scratch.0.join("state").into()),
        ("CHARIOX_DAEMON_ID", "recorded-claude-model-usage".into()),
        ("CHARIOX_CLAUDE_BIN", claude.into()),
        ("CHARIOX_MODEL_USAGE_FIXTURE", record.clone().into()),
        ("CLAUDE_CONFIG_DIR", config.into()),
    ];
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let first_kernel = real_kernel::start(&scratch.0.join("first"), &env, None);
    let (room, agent) = runtime.block_on(async {
        let mut socket = connect(&first_kernel).await;
        let room = create_session(&mut socket, &scratch.0).await;
        let spawned = request(
            &mut socket,
            json!({"SpawnAgent": {"session_id": room, "alias": "claude-usage",
                "provider": "claude", "model": "claude-haiku-5-5", "effort": null,
                "worktree_id": null, "no_subscribe": true}}),
        )
        .await
        .unwrap();
        let agent = spawned["AgentSpawned"]["agent"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let attachment = attach_claude_usage(&mut socket, &room).await;
        claude_usage_turn(&mut socket, &room, &agent, &attachment, &record, 1).await;
        assert_eq!(
            model_inputs(&mut socket, &room, &agent).await,
            [
                ("claude-haiku-5-5".into(), 100),
                ("claude-opus-4-6".into(), 40),
            ]
        );
        claude_usage_turn(&mut socket, &room, &agent, &attachment, &record, 2).await;
        assert_eq!(
            model_inputs(&mut socket, &room, &agent).await,
            [
                ("claude-haiku-5-5".into(), 250),
                ("claude-opus-4-6".into(), 40),
                ("claude-sonnet-4-6".into(), 20),
            ]
        );
        (room, agent)
    });
    drop(first_kernel);
    let second_kernel = real_kernel::start(&scratch.0.join("second"), &env, None);
    runtime.block_on(async {
        let mut socket = connect(&second_kernel).await;
        let attachment = attach_claude_usage(&mut socket, &room).await;
        claude_usage_turn(&mut socket, &room, &agent, &attachment, &record, 3).await;
        let first = std::fs::read_to_string(record.join("turn-1")).unwrap();
        let resumed = std::fs::read_to_string(record.join("turn-3")).unwrap();
        assert_eq!(
            resumed,
            format!("{} True", first.split_whitespace().next().unwrap())
        );
        assert_eq!(
            model_inputs(&mut socket, &room, &agent).await,
            [
                ("claude-haiku-5-5".into(), 310),
                ("claude-opus-4-6".into(), 40),
                ("claude-sonnet-4-6".into(), 20),
            ]
        );
        claude_usage_turn(&mut socket, &room, &agent, &attachment, &record, 4).await;
        assert_eq!(
            model_inputs(&mut socket, &room, &agent).await,
            [
                ("claude-haiku-5-5".into(), 325),
                ("claude-opus-4-6".into(), 40),
                ("claude-sonnet-4-6".into(), 20),
            ]
        );
    });
}

async fn attach_claude_usage(socket: &mut Socket, room: &str) -> String {
    let response = request(
        socket,
        json!({"AttachToSession": {"session_id": room,
        "client_id": "claude-model-usage-test", "capability_level": "FullTerminal"}}),
    )
    .await
    .unwrap();
    response["SessionAttached"]["attachment"]["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

async fn claude_usage_turn(
    socket: &mut Socket,
    room: &str,
    agent: &str,
    attachment: &str,
    record: &std::path::Path,
    turn: u32,
) {
    request(
        socket,
        json!({"SubmitPrompt": {"session_id": room, "attachment_id": attachment,
        "target_agent_id": agent, "prompt": format!("usage turn {turn}"), "attachments": []}}),
    )
    .await
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let response = request(socket, json!({"GetSessionState": {"session_id": room}}))
            .await
            .unwrap();
        if record.join(format!("turn-{turn}")).exists()
            && response["SessionState"]["session"]["prompt_states"][agent]["active_prompt"]
                .is_null()
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Claude turn {turn} did not settle"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn model_inputs(socket: &mut Socket, room: &str, agent: &str) -> Vec<(String, u64)> {
    usage(socket, room, agent)
        .await
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            (
                row["model"].as_str().unwrap().to_owned(),
                row["input_tokens"].as_u64().unwrap(),
            )
        })
        .collect()
}
