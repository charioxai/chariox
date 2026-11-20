//! Exercise provider error handling through the app-server protocol and the
//! owner's usual terminal interaction.
use super::*;

const POLICY_REASON: &str = "The provider declined this request under its content policy.";
const POLICY_RECOVERY: &str =
    "Reword the request and retry, or switch this agent to another provider.";

#[test]
fn a_codex_content_policy_card_uses_the_normal_owner_interaction() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            for (alias, code, kind, diagnostic, expected, action) in [
                ("policy-resume", "cyberPolicy", "content_policy", "Private refusal detail",
                    POLICY_REASON, "resume"),
                ("policy-cancel", "cyberPolicy", "content_policy", "Private refusal detail",
                    POLICY_REASON, "cancel"),
                ("transient", "internalServerError", "other", "Temporary server problem",
                    "The provider could not complete this request. Send the request again.", "cancel"),
                ("account", "modelNotFound", "other", "Model access is disabled for this account",
                    "The provider could not complete this request. Check the provider account and model settings, then send the request again.", "cancel"),
            ] {
                let (kernel, record) = start_case(alias);
                verify_case(&kernel, &record, alias, code, kind, diagnostic, expected, action).await;
            }
        });
}

fn start_case(alias: &str) -> (Kernel, PathBuf) {
    let scratch = std::env::temp_dir().join(format!("hr-policy-{}-{alias}", std::process::id()));
    let record = scratch.join("record");
    std::fs::create_dir_all(&record).unwrap();
    std::fs::create_dir_all(scratch.join("codex-home")).unwrap();
    let script = scratch.join("fake_codex.py");
    std::fs::write(&script, FAKE_CODEX).unwrap();
    let codex = scratch.join("codex");
    install(
        |file| {
            std::fs::write(
                file,
                format!("#!/bin/sh\nexec python3 {} \"$@\"\n", script.display()),
            )
            .unwrap();
        },
        &codex,
    );
    let kernel = start_laptop_kernel(
        &scratch,
        &[
            ("CHARIOX_CODEX_BIN", codex),
            ("CHARIOX_FAKE_CODEX_RECORD", record.clone()),
            ("CODEX_HOME", scratch.join("codex-home")),
            ("CHARIOX_ROOM_AGENT_TOOLS", "1".into()),
        ],
    );
    (kernel, record)
}

#[expect(
    clippy::too_many_arguments,
    reason = "Keep each case's fixture and expected outcome explicit"
)]
async fn verify_case(
    kernel: &Kernel,
    record: &Path,
    alias: &str,
    code: &str,
    kind: &str,
    diagnostic: &str,
    expected: &str,
    action: &str,
) {
    let mut socket = connect(kernel, Duration::from_secs(60)).await;
    let project = kernel.scratch.join("project");
    let created = request(
        &mut socket,
        json!({"CreateSession": {
            "workspace_id": project, "worktree_id": project, "alias": null, "no_subscribe": true
        }}),
    )
    .await;
    let room = created["SessionCreated"]["session"]["id"].as_str().unwrap();
    let spawned = request(
        &mut socket,
        json!({"SpawnAgent": {
            "session_id": room, "alias": alias, "provider": "codex", "model": "gpt-6-luna",
            "effort": "low", "worktree_id": null, "no_subscribe": true
        }}),
    )
    .await;
    let agent = spawned["AgentSpawned"]["agent"]["id"].as_str().unwrap();
    let attached = request(
        &mut socket,
        json!({"AttachToSession": {
            "session_id": room, "client_id": alias, "capability_level": "FullTerminal"
        }}),
    )
    .await;
    let attachment = attached["SessionAttached"]["attachment"]["id"]
        .as_str()
        .unwrap();
    let request_path = record.join("turn-failure");
    let release_path = record.join("report-failure");
    std::fs::write(
        &request_path,
        json!({"message": diagnostic, "codexErrorInfo": code}).to_string(),
    )
    .unwrap();
    request(
        &mut socket,
        json!({"SubmitPrompt": {
            "session_id": room, "target_agent_id": agent, "attachment_id": attachment,
            "prompt": "Review the change", "attachments": []
        }}),
    )
    .await;
    let deadline = Instant::now() + Duration::from_secs(60);
    let (prompt, run) = loop {
        let state = request(
            &mut socket,
            json!({"GetSessionState": {"session_id": room}}),
        )
        .await;
        let prompt = &state["SessionState"]["session"]["prompt_states"][agent]["active_prompt"];
        if let (Some(prompt_id), Some(run)) = (
            prompt["id"].as_str(),
            state["SessionState"]["agent_activity"][agent]["active_turn"]["provider_run_id"]
                .as_str(),
        ) {
            break (prompt_id.to_string(), run.to_string());
        }
        assert!(Instant::now() < deadline, "the provider never started");
        tokio::time::sleep(Duration::from_millis(100)).await;
    };

    // An armed timer leaves an obligation, so the failure raises the owner card.
    let started = codex_tools(
        record,
        alias,
        json!({"jsonrpc": "2.0", "id": 1,
        "method": "tools/call", "params": {"name": "chariox.events.timer", "arguments": {
            "task_id": prompt, "origin_prompt_id": prompt,
            "delay_ms": 600_000, "label": "Policy decline drill"
        }}}),
    )
    .await;
    std::fs::remove_file(record.join(format!("mcp-{alias}"))).unwrap();
    let result: Value = serde_json::from_str(
        started["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_else(|| panic!("{started}")),
    )
    .unwrap();
    assert!(result["registration_id"].is_string(), "{result}");
    std::fs::write(&release_path, "").unwrap();

    let deadline = Instant::now() + Duration::from_secs(60);
    let (card, outcome) = loop {
        let state = request(
            &mut socket,
            json!({"GetSessionState": {"session_id": room}}),
        )
        .await;
        let card = state["SessionState"]["session"]["active_interactions"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|card| {
                card["id"]
                    .as_str()
                    .is_some_and(|id| id.starts_with("task-blocked-"))
            });
        let outcome =
            &state["SessionState"]["agent_activity"][agent]["last_completed_turn"]["outcome"];
        if let Some(card) = card.filter(|_| !outcome.is_null()) {
            break (card.clone(), outcome.clone());
        }
        assert!(Instant::now() < deadline, "the card never arrived");
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let message = card["message"].as_str().unwrap();
    assert!(message.contains(expected), "{message}");
    assert!(!message.contains(diagnostic));
    assert_eq!(outcome["kind"], kind, "{alias}");
    assert_eq!(
        card["choices"]
            .as_array()
            .unwrap()
            .iter()
            .map(|choice| choice["label"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["Cancel", "Resume"]
    );
    if kind == "content_policy" {
        assert!(message.contains(POLICY_RECOVERY), "{message}");
        assert!(!message.contains("provider account"));
        assert_eq!(outcome["reason"], POLICY_REASON);
        assert_eq!(outcome["retryable"], false);
        assert!(std::fs::read_dir(kernel.scratch.join("logs"))
            .unwrap()
            .any(|file| {
                std::fs::read_to_string(file.unwrap().path())
                    .unwrap()
                    .lines()
                    .any(|line| {
                        line.contains(diagnostic) && line.contains(agent) && line.contains(&run)
                    })
            }));
    }

    std::fs::remove_file(&request_path).unwrap();
    std::fs::remove_file(&release_path).unwrap();
    std::fs::write(record.join("go"), "").unwrap();
    request(
        &mut socket,
        json!({"RespondToInteraction": {
            "session_id": room, "interaction_id": card["id"], "choice_id": action
        }}),
    )
    .await;
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let state = request(
            &mut socket,
            json!({"GetSessionState": {"session_id": room}}),
        )
        .await;
        let current = &state["SessionState"]["session"]["prompt_states"][agent]["active_prompt"];
        let tasks = state["SessionState"]["session"]["agent_tasks"]
            .as_array()
            .unwrap();
        let task = tasks.iter().find(|task| task["task_id"] == prompt);
        if action == "resume" && current["id"].as_str().is_some_and(|id| id != prompt) {
            break;
        }
        if action == "cancel" && task.is_some_and(|task| task["state"] == "cancelled") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the {action} did not settle the card"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
