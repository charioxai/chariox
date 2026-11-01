//! MP-08 / MP-09 / MP-10 / MP-11 A12-S05: protected assessment control boundary.
use super::*;

#[tokio::test]
async fn a12_assessment_exact_inactive_id_cannot_cross_the_protected_boundary() {
    use crate::durable_state::agent_lifecycle::{Operation, Registration};
    let (_app, runtime, room, agent) = agent_config_runtime().await;
    let store = &runtime.owned.durable_state_store;
    let inactive = "completion-wake-abc";
    for op in [
        Operation::Begin {
            owner: "owner".into(),
            room: room.clone(),
            agent: agent.clone(),
            prompt: "p".into(),
            run: Some("run".into()),
            now: 1,
        },
        Operation::StartGoal {
            task: "p".into(),
            owner: "owner".into(),
            objective: "Wait only on the selected source".into(),
            minutes: 10,
            now: 1,
        },
    ] {
        store.agent_lifecycle(op).unwrap();
    }
    for id in [inactive, "completion-wake-abcd"] {
        store
            .agent_lifecycle(Operation::Subscribe {
                task: "p".into(),
                prompt: "p".into(),
                registration: Registration {
                    id: id.into(),
                    task_id: "p".into(),
                    source_id: id.into(),
                    obligation_id: None,
                    source_cursor: 0,
                    live: true,
                },
            })
            .unwrap();
    }
    store
        .agent_lifecycle(Operation::Unsubscribe {
            task: "p".into(),
            prompt: "p".into(),
            registration: inactive.into(),
        })
        .unwrap();
    store
        .agent_lifecycle(Operation::Settle {
            room: room.clone(),
            agent: agent.clone(),
            prompt: "p".into(),
            run: "run".into(),
            has_answer: true,
            cancelled: false,
            now: 2,
        })
        .unwrap();
    let check = store
        .agent_tasks(Some(&room), Some(&agent))
        .unwrap()
        .remove(0)
        .pending_prompt_id
        .unwrap();
    store
        .agent_lifecycle(Operation::BeginContinuation {
            task: "p".into(),
            room: room.clone(),
            agent: agent.clone(),
            prompt: check.clone(),
            run: Some("run".into()),
            now: 3,
        })
        .unwrap();
    runtime
        .owned
        .room_secret_observations
        .register(&room, "PRIVATE_CANARY")
        .unwrap();
    let result=serde_json::json!({"status":"waiting","reason":"PRIVATE_CANARY","registration_ids":[inactive],"deadline_ms":60_000}).to_string();
    runtime.owned.private_assessment_results.append(
        &room,
        &agent,
        &check,
        "run",
        result.as_bytes(),
    );
    assert!(
        runtime
            .owned
            .take_private_assessment(&room, &agent, &check, "run")
            .is_empty(),
        "the fence must preserve a known inactive identity instead of rebinding it"
    );
}

#[tokio::test]
async fn a12_assessment_unique_live_prefix_crosses_the_protected_boundary() {
    crate::test_support::isolated_env_test!();
    use crate::durable_state::agent_lifecycle::{ExecutionState, Operation, Registration};
    use crate::terminal::TerminalOutputKind;
    let trace_dir = std::env::temp_dir().join(format!(
        "chariox-assessment-trace-{:032x}",
        rand::random::<u128>()
    ));
    std::env::remove_var("CHARIOX_DISABLE_TERMINAL_TURN_TRACE");
    std::env::remove_var("CHARIOX_TERMINAL_TURN_TRACE");
    std::env::set_var("CHARIOX_TERMINAL_TURN_TRACE_DIR", &trace_dir);
    let mut config = crate::config::DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let (_app, runtime, room, agent) =
        agent_config_runtime_with_config_and_owner(config, "owner").await;
    let now = crate::session::unix_epoch_ms();
    let store = &runtime.owned.durable_state_store;
    let full = "completion-wake-a9eedf22677192661f03c4b44171fc0b";
    for op in [
        Operation::Begin {
            owner: "owner".into(),
            room: room.clone(),
            agent: agent.clone(),
            prompt: "p".into(),
            run: Some("run".into()),
            now,
        },
        Operation::StartGoal {
            task: "p".into(),
            owner: "owner".into(),
            objective: "Wait for the original timer".into(),
            minutes: 10,
            now,
        },
        Operation::Subscribe {
            task: "p".into(),
            prompt: "p".into(),
            registration: Registration {
                id: full.into(),
                task_id: "p".into(),
                source_id: "timer".into(),
                obligation_id: None,
                source_cursor: 0,
                live: true,
            },
        },
        Operation::Settle {
            room: room.clone(),
            agent: agent.clone(),
            prompt: "p".into(),
            run: "run".into(),
            has_answer: true,
            cancelled: false,
            now,
        },
    ] {
        store.agent_lifecycle(op).unwrap();
    }
    let task = store
        .agent_tasks(Some(&room), Some(&agent))
        .unwrap()
        .remove(0);
    let check = task.pending_prompt_id.clone().unwrap();
    runtime
        .owned
        .goal_continuation_context(&task)
        .unwrap()
        .unwrap();
    assert!(!trace_dir.exists(), "diagnostics are disabled by default");
    std::env::set_var("CHARIOX_TERMINAL_TURN_TRACE", "1");
    runtime
        .owned
        .goal_continuation_context(&task)
        .unwrap()
        .unwrap();
    store
        .agent_lifecycle(Operation::BeginContinuation {
            task: "p".into(),
            room: room.clone(),
            agent: agent.clone(),
            prompt: check.clone(),
            run: Some("run".into()),
            now,
        })
        .unwrap();
    runtime
        .owned
        .room_secret_observations
        .register(&room, "PRIVATE_CANARY")
        .unwrap();
    let prompt = crate::session::PromptQueueItem::new(
        &check,
        "attachment-1",
        &agent,
        "check",
        crate::session::PromptStatus::Running,
    )
    .with_prompt_origin(crate::session::PromptOrigin::CompletionCheck);
    runtime
        .owned
        .prompt_state_owner
        .sync_external_active_prompt(
            &runtime.owned.session_store.get_session(&room).unwrap(),
            &agent,
            Some(prompt.clone()),
        );
    let shortened = &full[..full.len() - 1];
    let raw = serde_json::json!({"status":"waiting","reason":"PRIVATE_CANARY PRIVATE_PROVIDER_PROSE","registration_ids":[shortened],"deadline_ms":now+60_000}).to_string();
    runtime.owned.fan_out_terminal_outputs(
        &room,
        vec![
            crate::runtime::state::prompt_transcript_owned_state::TerminalOutputBatchAppend {
                sequence: None,
                provider_run_id: "run".into(),
                agent_id: Some(agent.clone()),
                kind: TerminalOutputKind::ProviderOutput,
                merge_key: Some("assessment".into()),
                bytes: raw.as_bytes().to_vec(),
            },
        ],
    );
    let (settled, _) = runtime
        .owned
        .settle_agent_task(&room, &agent, &prompt, "run", false)
        .unwrap()
        .unwrap();
    assert_eq!(
        settled.state,
        ExecutionState::Waiting,
        "a unique live reference survives the private fence: {}",
        settled.reason
    );
    assert_eq!(
        settled.wait.as_ref().unwrap().registration_ids,
        vec![full.to_string()]
    );
    let receipt = serde_json::to_string(&settled).unwrap();
    assert!(!receipt.contains("PRIVATE_"));
    let trace_file = std::fs::read_dir(&trace_dir)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let trace = std::fs::read_to_string(&trace_file).unwrap();
    let records = trace
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    let input = records
        .iter()
        .find(|record| record["source"] == "goal_assessment_input")
        .unwrap();
    assert_eq!(input["payload"]["registrations"][0]["id"], full);
    let output = records
        .iter()
        .find(|record| record["source"] == "goal_assessment_waiting_reference")
        .unwrap();
    assert_eq!(
        output["payload"]["references"][0]["returned_reference"],
        shortened
    );
    assert_eq!(
        input["payload"]["prompt_id"],
        output["payload"]["prompt_id"]
    );
    assert_eq!(
        input["payload"]["objective_revision"],
        output["payload"]["objective_revision"]
    );
    assert!(
        !trace.contains("PRIVATE_"),
        "private prose never enters diagnostics"
    );
    std::env::remove_var("CHARIOX_TERMINAL_TURN_TRACE");
    std::env::remove_var("CHARIOX_TERMINAL_TURN_TRACE_DIR");
    std::fs::remove_dir_all(trace_dir).unwrap();
}
