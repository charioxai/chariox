//! A kernel that requires its host's token (managed and hosted workers) still
//! starts the local HTTP gateway of an agent trigger: the gateway reads the
//! credential the way its real client does and the kernel admits it.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::json;

#[path = "support/real_kernel.rs"]
mod real_kernel;
use real_kernel::{connect, create_session, request};

const STUB_CLI: &str = include_str!("agent_trigger_gateway/stub_cli.py");
const FAKE_CODEX: &str = include_str!("hot_reload/fake_codex.py");

fn write_executable(path: &Path, contents: &str) {
    std::fs::write(path, contents).unwrap();
    std::fs::set_permissions(path, PermissionsExt::from_mode(0o755)).unwrap();
}

#[test]
fn a_kernel_that_requires_a_host_token_starts_the_http_trigger_gateway() {
    let scratch = std::env::temp_dir().join(format!("atg-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    for directory in ["bin", "codex-home", "record"] {
        std::fs::create_dir_all(scratch.join(directory)).unwrap();
    }
    write_executable(&scratch.join("bin/chariox-cli"), STUB_CLI);
    std::fs::write(scratch.join("fake_codex.py"), FAKE_CODEX).unwrap();
    write_executable(
        &scratch.join("bin/codex"),
        &format!(
            "#!/bin/sh\nexec python3 {} \"$@\"\n",
            scratch.join("fake_codex.py").display()
        ),
    );
    let kernel = real_kernel::start(
        &scratch,
        &[
            ("CHARIOX_CLI_BIN", scratch.join("bin/chariox-cli").into()),
            ("CHARIOX_CODEX_BIN", scratch.join("bin/codex").into()),
            ("CHARIOX_FAKE_CODEX_RECORD", scratch.join("record").into()),
            ("CODEX_HOME", scratch.join("codex-home").into()),
        ],
        None,
    );
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let mut socket = connect(&kernel).await;
            let session_id = create_session(&mut socket, &scratch).await;
            let spawned = request(
                &mut socket,
                json!({"SpawnAgent": {"session_id": session_id, "alias": "writer",
                    "provider": "codex", "model": "gpt-6-luna", "effort": "low",
                    "worktree_id": null, "no_subscribe": true}}),
            )
            .await
            .unwrap();
            let agent_id = spawned["AgentSpawned"]["agent"]["id"]
                .as_str()
                .unwrap_or_else(|| panic!("{spawned}"))
                .to_string();
            // A trigger reaches a running agent: start its turn.
            let attached = request(
                &mut socket,
                json!({"AttachToSession": {"session_id": session_id,
                    "client_id": "trigger-gateway-test", "capability_level": "FullTerminal"}}),
            )
            .await
            .unwrap();
            let attachment_id = attached["SessionAttached"]["attachment"]["id"]
                .as_str()
                .unwrap_or_else(|| panic!("{attached}"))
                .to_string();
            request(
                &mut socket,
                json!({"SubmitPrompt": {"session_id": session_id, "attachment_id": attachment_id,
                    "target_agent_id": agent_id, "prompt": "write four hundred parts",
                    "attachments": []}}),
            )
            .await
            .unwrap();

            let trigger = request(
                &mut socket,
                json!({"CreateAgentTrigger": {"session_id": session_id, "agent_id": agent_id,
                    "source": {"kind": "http"}, "busy_policy": "queue",
                    "prompt": "Decide what to do"}}),
            )
            .await
            .unwrap_or_else(|error| panic!("the gateway did not start: {error}"));
            let trigger_id = trigger["AgentTriggerCreated"]["trigger"]["id"]
                .as_str()
                .unwrap_or_else(|| panic!("{trigger}"))
                .to_string();

            // What the gateway forwards from a POST.
            let deadline = Instant::now() + Duration::from_secs(60);
            loop {
                let invoked = request(
                    &mut socket,
                    json!({"InvokeAgentTrigger": {"session_id": session_id,
                        "trigger_id": trigger_id, "input": {"event": "pushed"}}}),
                )
                .await
                .unwrap();
                match invoked["AgentTriggerInvoked"]["invocation"]["status"].as_str() {
                    Some("accepted") => break,
                    Some("offline") if Instant::now() < deadline => {
                        tokio::time::sleep(Duration::from_millis(200)).await
                    }
                    _ => panic!("the agent did not take the trigger: {invoked}"),
                }
            }
        });
    let leftovers: Vec<_> = std::fs::read_dir(scratch.join("tmp"))
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name())
        .filter(|name| {
            name.to_string_lossy()
                .starts_with("chariox-agent-trigger-auth")
        })
        .collect();
    assert!(
        leftovers.is_empty(),
        "no credential file is left behind: {leftovers:?}"
    );
}
