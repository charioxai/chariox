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
    spawn_with_permission(socket, room, alias, "required").await
}

async fn spawn_with_permission(
    socket: &mut Socket,
    room: &str,
    alias: &str,
    permission: &str,
) -> String {
    let response = request(
        socket,
        json!({"SpawnAgent": {"session_id": room, "alias": alias,
            "provider": "dev-stub", "model": "trace", "permission_level": permission,
            "no_subscribe": true}}),
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
    // The trace provider launches Node.js from the test runner's installation. Only `node` joins
    // the system PATH: the provider CLIs a runner installs would be discovered and validated in
    // the background, and that discovery is still running when the shutdown test stops the kernel.
    let path = real_kernel::node_path(scratch);
    real_kernel::start(
        scratch,
        &[
            ("PATH", path),
            ("CHARIOX_PROVIDER_DEV_STUB", "1".into()),
            ("CHARIOX_ROOM_AGENT_TOOLS", "1".into()),
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
    for (alias, lifecycle, outcome) in [
        ("orch", "completed", "completed"),
        ("cancelled", "cancelled", "cancelled"),
        ("failed", "failed", "other"),
    ] {
        let agent = spawn(&mut socket, &room, alias).await;
        submit(&mut socket, &room, &agent).await.unwrap();
        let turn = completed_turn(&mut socket, &room, &agent).await;
        assert_eq!(turn["lifecycle"], lifecycle, "{turn}");
        assert_eq!(turn["outcome"]["kind"], outcome, "{turn}");
        assert_eq!(
            turn["user_prompt"]["entry"]["kind"], "user_prompt",
            "{turn}"
        );
        assert_eq!(
            turn["summary"]["entry"]["kind"], "provider_output",
            "{turn}"
        );
        assert_eq!(turn["summary"]["total_chars"], 120, "{turn}");
        let blobs = turn["blobs"].as_array().unwrap();
        let rows = |kind: &str| {
            blobs
                .iter()
                .filter(|blob| blob["kind"] == kind)
                .map(|blob| blob["entry_count"].as_u64().unwrap())
                .sum::<u64>()
        };
        assert_eq!(rows("provider_output"), 0, "{turn}");
        assert_eq!(rows("provider_reasoning"), 1, "{turn}");
        assert_eq!(rows("provider_tool"), 2, "{turn}");
    }
}

#[tokio::test]
async fn trace_spawns_two_levels_of_generated_agents() {
    let scratch = std::env::temp_dir().join(format!("loadlab-spawn-{}", std::process::id()));
    let kernel = start(&scratch);
    for (alias, child, chars) in [
        ("orch", Some("child"), 51),
        ("child", Some("grandchild"), 83),
        ("grandchild", None, 137),
    ] {
        let mut records = vec![
            json!({"v": 1, "source": "generator", "seed": 42, "agent": alias, "notes": ""}),
            json!({"t": 0, "kind": "prompt", "chars": 30}),
        ];
        if let Some(child) = child {
            records.push(
                json!({"t": 0, "kind": "spawn", "agent": child, "trace": format!("{child}.jsonl")}),
            );
        }
        records.extend([
            json!({"t": 0, "kind": "text", "chars": chars, "stream_ms": 0, "markdown": "plain"}),
            json!({"t": 0, "kind": "end", "outcome": "completed"}),
        ]);
        let fixture = records
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(scratch.join(format!("{alias}.jsonl")), fixture).unwrap();
    }

    let mut socket = connect(&kernel).await;
    let room = create_session(&mut socket, &scratch).await;
    let root = spawn(&mut socket, &room, "orch").await;
    submit(&mut socket, &room, &root).await.unwrap();
    let first = completed_turn(&mut socket, &room, &root).await;
    assert_eq!(first["lifecycle"], "completed", "{first}");
    assert_eq!(first["summary"]["total_chars"], 51, "{first}");

    let mut parent = root.clone();
    for (alias, chars, depth) in [("child", 83, 1), ("grandchild", 137, 2)] {
        let tree = request(
            &mut socket,
            json!({"ListDescendants": {"session_id": room, "agent_id": root}}),
        )
        .await
        .unwrap();
        let child = tree["AgentTree"]["tree"]["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|child| child["alias"] == alias)
            .unwrap_or_else(|| panic!("missing {alias}: {tree}"));
        assert_eq!(child["parent_agent_id"], parent, "{tree}");
        assert_eq!(child["depth"], depth, "{tree}");
        let id = child["agent_id"].as_str().unwrap();
        let turn = completed_turn(&mut socket, &room, id).await;
        assert_eq!(turn["lifecycle"], "completed", "{turn}");
        assert_eq!(
            turn["summary"]["entry"]["kind"], "provider_output",
            "{turn}"
        );
        assert_eq!(
            turn["summary"]["total_chars"], chars,
            "{alias} did not run its own trace: {turn}"
        );
        parent = id.to_owned();
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

#[tokio::test]
async fn trace_uses_real_permission_terminal_and_inbox() {
    let scratch = std::env::temp_dir().join(format!("loadlab-effects-{}", std::process::id()));
    let kernel = start(&scratch);
    std::fs::write(
        scratch.join("orch.jsonl"),
        concat!(
            r#"{"v":1,"source":"generator","seed":42,"agent":"orch","notes":"side effects"}"#,
            "\n",
            r#"{"t":0,"kind":"prompt","chars":20}"#,
            "\n",
            r#"{"t":0,"kind":"permission","tool":"Bash"}"#,
            "\n",
            r#"{"t":0,"kind":"terminal","lines":24,"line_bytes":80,"rate_lps":4000}"#,
            "\n",
            r#"{"t":0,"kind":"notify","source":"lab-notice","count":2,"interval_ms":20}"#,
            "\n",
            r#"{"t":0,"kind":"text","chars":120,"stream_ms":0,"markdown":"plain"}"#,
            "\n",
            r#"{"t":0,"kind":"end","outcome":"completed"}"#,
            "\n",
        ),
    )
    .unwrap();
    let mut socket = connect(&kernel).await;
    let room = create_session(&mut socket, &scratch).await;
    let agent = spawn(&mut socket, &room, "orch").await;
    submit(&mut socket, &room, &agent).await.unwrap();

    let deadline = Instant::now() + Duration::from_secs(30);
    let mut answered = std::collections::BTreeSet::new();
    loop {
        let inbox = request(&mut socket, json!({"GetPersonInbox": null}))
            .await
            .unwrap();
        for card in inbox["PersonInbox"]["cards"].as_array().unwrap() {
            if card["session_id"] != room {
                continue;
            }
            let interaction = &card["body"]["needs_you"]["interaction"];
            let Some(choices) = interaction["choices"].as_array() else {
                continue;
            };
            let card_id = card["card_id"].as_str().unwrap();
            if !answered.insert(card_id.to_owned()) {
                continue;
            }
            let choice = choices
                .iter()
                .find(|choice| choice["id"] == "allow_once")
                .or_else(|| {
                    choices.iter().find(|choice| {
                        choice["label"]
                            .as_str()
                            .is_some_and(|label| label.starts_with("Allow"))
                    })
                })
                .unwrap_or_else(|| panic!("no allow choice: {interaction}"));
            request(
                &mut socket,
                json!({"RespondToInteraction": {
                    "session_id": room, "interaction_id": card_id, "choice_id": choice["id"],
                }}),
            )
            .await
            .unwrap();
        }
        let response = request(
            &mut socket,
            json!({"GetSessionHistoryOutline": {
                "session_id": room, "agent_ids": [agent],
            }}),
        )
        .await
        .unwrap();
        let turn = &response["SessionHistoryOutline"]["agents"][0]["turns"][0];
        if let Some("completed" | "cancelled" | "failed") = turn["lifecycle"].as_str() {
            assert_eq!(turn["lifecycle"], "completed", "{turn}");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "side effects did not settle after {answered:?}: {inbox}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(!answered.is_empty(), "expected a real permission decision");

    let list = real_kernel::terminal(&mut socket, &room, json!({"operation": "list"}))
        .await
        .unwrap();
    let terminal = list["terminals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|terminal| terminal["title"] == "Load lab terminal")
        .unwrap_or_else(|| panic!("no generated Room terminal: {list}"));
    let output = real_kernel::terminal(
        &mut socket,
        &room,
        json!({"operation": "wait", "terminal_id": terminal["terminal_id"],
            "watches": [{"kind": "finished"}], "timeout_ms": 10000}),
    )
    .await
    .unwrap();
    let waited = &output["waited"];
    assert_eq!(waited["fired"]["kind"], "finished", "{output}");
    assert_eq!(waited["fired"]["exit_code"], 0, "{output}");
    assert!(
        waited["screen_text"]
            .as_str()
            .unwrap()
            .contains("Generated lab text"),
        "{output}"
    );
    assert!(
        waited["output_offset"].as_u64().unwrap() >= 24 * 80,
        "{output}"
    );
    let inbox = request(&mut socket, json!({"GetPersonInbox": null}))
        .await
        .unwrap();
    let notices = inbox["PersonInbox"]["cards"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|card| {
            card["session_id"] == room
                && card["body"]["notice"]["title"]
                    .as_str()
                    .is_some_and(|title| title.starts_with("lab-notice "))
        })
        .count();
    assert_eq!(notices, 2, "{inbox}");
}

#[tokio::test]
async fn trace_applies_session_permissions_before_asking() {
    let scratch = std::env::temp_dir().join(format!("loadlab-permissions-{}", std::process::id()));
    let kernel = start(&scratch);
    let mut socket = connect(&kernel).await;
    let room = create_session(&mut socket, &scratch).await;

    for (alias, permission, expected_cards) in [("allowed", "yolo", 0), ("granted", "required", 1)]
    {
        let prompt = r#"{"t":0,"kind":"prompt","chars":30}"#;
        let ask = r#"{"t":0,"kind":"permission","tool":"Bash"}"#;
        let fixture = FIXTURE
            .replace(r#""agent":"orch""#, &format!(r#""agent":"{alias}""#))
            .replace(prompt, &format!("{prompt}\n{ask}\n{ask}"));
        std::fs::write(scratch.join(format!("{alias}.jsonl")), fixture).unwrap();
        let agent = spawn_with_permission(&mut socket, &room, alias, permission).await;
        submit(&mut socket, &room, &agent).await.unwrap();

        let deadline = Instant::now() + Duration::from_secs(20);
        let mut answered = std::collections::BTreeSet::new();
        loop {
            let inbox = request(&mut socket, json!({"GetPersonInbox": null}))
                .await
                .unwrap();
            for card in inbox["PersonInbox"]["cards"].as_array().unwrap() {
                if card["session_id"] != room {
                    continue;
                }
                let interaction = &card["body"]["needs_you"]["interaction"];
                let Some(choices) = interaction["choices"].as_array() else {
                    continue;
                };
                if !choices.iter().any(|choice| choice["id"] == "allow_session") {
                    continue;
                }
                let id = card["card_id"].as_str().unwrap();
                if !answered.insert(id.to_owned()) {
                    continue;
                }
                assert_eq!(
                    answered.len(),
                    expected_cards,
                    "unexpected permission request: {interaction}"
                );
                request(
                    &mut socket,
                    json!({"RespondToInteraction": {
                        "session_id": room, "interaction_id": id, "choice_id": "allow_session",
                    }}),
                )
                .await
                .unwrap();
            }
            let history = request(
                &mut socket,
                json!({"GetSessionHistoryOutline": {
                    "session_id": room, "agent_ids": [agent],
                }}),
            )
            .await
            .unwrap();
            let turn = &history["SessionHistoryOutline"]["agents"][0]["turns"][0];
            if let Some("completed" | "cancelled" | "failed") = turn["lifecycle"].as_str() {
                assert_eq!(turn["lifecycle"], "completed", "{turn}");
                break;
            }
            assert!(
                Instant::now() < deadline,
                "permission replay did not settle: {inbox}"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert_eq!(answered.len(), expected_cards);
    }
}

#[tokio::test]
#[ignore = "S1 shapes a full 40-agent Room and is run as a separate drill"]
async fn trace_seeds_s1_through_the_real_kernel() {
    use std::process::Stdio;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let scratch = std::env::temp_dir().join(format!("loadlab-s1-{}", std::process::id()));
    let kernel = start(&scratch);
    let mut socket = connect(&kernel).await;
    let room = create_session(&mut socket, &scratch).await;
    let attached = request(
        &mut socket,
        json!({"AttachToSession": {"session_id": room, "client_id": "loadlab-s1",
            "capability_level": "FullTerminal"}}),
    )
    .await
    .unwrap();
    let attachment = attached["SessionAttached"]["attachment"]["id"]
        .as_str()
        .unwrap();

    let script = r#"
const { once } = require('node:events')
const { writeFileSync } = require('node:fs')
const { join } = require('node:path')
const { createInterface } = require('node:readline')
const { generate } = require('./generate.cjs')
const { seedS1 } = require('./seed-s1.cjs')
const [directory, sessionId, attachmentId] = process.argv.slice(1)
const input = createInterface({ input: process.stdin })
const traces = generate('s1', 42)
for (const [filename, contents] of traces) writeFileSync(join(directory, filename), contents)
const rpc = async request => {
  const reply = once(input, 'line')
  console.log(JSON.stringify({ request }))
  const [line] = await reply
  const { ok, value } = JSON.parse(line)
  if (!ok) throw new Error(JSON.stringify(value))
  return value
}
seedS1({ rpc, sessionId, attachmentId, traces })
  .then(({ tree, historyEntries }) => console.log(JSON.stringify({ done: {
    descendants: tree.nodes.length,
    states: Object.fromEntries(['running', 'waiting', 'idle'].map(state =>
      [state, tree.nodes.filter(node => node.state === state).length])),
    historyEntries: Object.values(historyEntries),
  } })))
  .catch(error => { console.error(error.message); process.exitCode = 1 })
  .finally(() => input.close())
"#;
    let mut driver = tokio::process::Command::new("node")
        .args(["-e", script])
        .arg(&scratch)
        .args([room.as_str(), attachment])
        .current_dir(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/drills/loadlab"
        ))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut incoming = BufReader::new(driver.stdout.take().unwrap()).lines();
    let mut outgoing = driver.stdin.take().unwrap();
    let result = tokio::time::timeout(Duration::from_secs(680), async {
        while let Some(line) = incoming.next_line().await.unwrap() {
            let frame: Value = serde_json::from_str(&line).unwrap();
            if let Some(body) = frame.get("request") {
                let response = match request(&mut socket, body.clone()).await {
                    Ok(value) => json!({"ok": true, "value": value}),
                    Err(value) => json!({"ok": false, "value": value}),
                };
                outgoing
                    .write_all(format!("{response}\n").as_bytes())
                    .await
                    .unwrap();
            } else {
                return frame["done"].clone();
            }
        }
        Value::Null
    })
    .await
    .expect("S1 did not finish its kernel requests");

    drop(outgoing);
    assert!(driver.wait().await.unwrap().success());
    assert_eq!(result["descendants"], 40, "{result}");
    assert_eq!(
        result["states"],
        json!({"running": 12, "waiting": 6, "idle": 22})
    );
    assert_eq!(result["historyEntries"].as_array().unwrap().len(), 22);
    println!("{result}");
}

#[tokio::test]
async fn shutdown_drains_a_trace_waiting_on_permission() {
    let scratch = std::env::temp_dir().join(format!("loadlab-shutdown-{}", std::process::id()));
    let mut kernel = start(&scratch);
    std::fs::write(
        scratch.join("waiting.jsonl"),
        concat!(
            r#"{"v":1,"source":"generator","seed":42,"agent":"waiting","notes":"pending permission"}"#, "\n",
            r#"{"t":0,"kind":"prompt","chars":24}"#, "\n",
            r#"{"t":0,"kind":"permission","tool":"Bash"}"#, "\n",
            r#"{"t":0,"kind":"end","outcome":"completed"}"#, "\n",
        ),
    ).unwrap();
    let mut socket = connect(&kernel).await;
    let room = create_session(&mut socket, &scratch).await;
    let agent = spawn(&mut socket, &room, "waiting").await;
    submit(&mut socket, &room, &agent).await.unwrap();

    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let inbox = request(&mut socket, json!({"GetPersonInbox": null}))
            .await
            .unwrap();
        let cards = inbox["PersonInbox"]["cards"].as_array().unwrap();
        if cards.iter().any(|card| {
            let interaction = &card["body"]["needs_you"]["interaction"];
            interaction["agent_id"] == agent && interaction["kind"] == "permission"
        }) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the trace never asked for permission"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let deadline = Instant::now() + Duration::from_secs(2);
    kernel.signal_shutdown();
    loop {
        if let Some(status) = kernel.try_wait() {
            assert!(status.success(), "{status}");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the kernel did not drain its provider in 2 seconds"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let log = std::fs::read_to_string(scratch.join("kernel.log")).unwrap();
    assert!(
        log.contains("kernel shutdown complete") && log.contains("abandoned: none"),
        "{log}"
    );
}
