//! MP-08 / MP-09 / MP-10 / MP-11: blocked restart and explicit unblock regressions.
use super::tests::runtime_with_active_prompt_config;
use super::*;
use crate::attachment::{AttachRequest, ClientCapabilityLevel};
use crate::config::DaemonConfig;

#[tokio::test]
async fn restart_recovery_preserves_blocked_turn_and_owner_decision() {
    use crate::durable_state::agent_lifecycle::{ExecutionState, Operation, Outcome};
    let mut config = DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let (runtime, room, agent, prompt) = runtime_with_active_prompt_config(
        crate::session::DurablePromptDeliveryPhase::Delivered,
        std::env::current_dir().unwrap().to_str().unwrap(),
        config,
    );
    let Outcome::Task(task) = runtime
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::Begin {
            owner: "local".into(),
            room: room.clone(),
            agent: agent.clone(),
            prompt: prompt.clone(),
            run: None,
            now: 1,
        })
        .unwrap()
    else {
        panic!("task should begin")
    };
    let Outcome::Task(blocked) = runtime
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::Block {
            task: task.task_id.clone(),
            prompt: prompt.clone(),
            reason: "Owner must approve the next step".into(),
        })
        .unwrap()
    else {
        panic!("task should block")
    };
    runtime
        .ensure_task_owner_interaction(blocked.clone())
        .await
        .unwrap();
    let id = format!(
        "task-blocked-{}-{}",
        blocked.task_id, blocked.blocked_revision
    );
    // Restart restores the projected card, but no process-local response channel.
    runtime.owned.pending_interactions.write().remove(&id);
    let targets = runtime.durable_restart_recovery_targets();
    let summary = runtime
        .drive_durable_restart_recovery(
            &targets,
            &BTreeSet::new(),
            std::time::Duration::from_millis(20),
            std::time::Duration::from_millis(1),
        )
        .await;
    assert_eq!(summary.failed_reconciliations, 0, "{summary:?}");
    assert_eq!(
        summary.unrecoverable_local_prompts_cancelled, 0,
        "{summary:?}"
    );
    assert_eq!(summary.provider_continuations_dispatched, 0);
    let kept = runtime
        .owned
        .durable_state_store
        .agent_tasks(Some(&room), Some(&agent))
        .unwrap()
        .remove(0);
    assert_eq!(kept.state, ExecutionState::Blocked);
    assert_eq!(kept.reason, blocked.reason);
    assert_eq!(kept.blocked_revision, blocked.blocked_revision);
    let session = runtime.owned.session_store.get_session(&room).unwrap();
    assert_eq!(
        runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &agent)
            .unwrap()
            .id(),
        prompt
    );
    assert!(
        runtime.owned.pending_interactions.write().contains_key(&id),
        "restored owner decision needs a live responder"
    );
    // Repeated recovery neither dispatches nor changes the owner decision identity.
    let again = runtime.recover_durable_runtime_after_restart().await;
    assert_eq!(again.failed_reconciliations, 0);
    runtime
        .resolve_terminal_runtime_interaction(&room, &id, "cancel", None, Some("local"))
        .await
        .unwrap();
    for _ in 0..100 {
        let task = runtime
            .owned
            .durable_state_store
            .agent_tasks(Some(&room), Some(&agent))
            .unwrap()
            .remove(0);
        if task.state == ExecutionState::Cancelled {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("the restored owner decision was not handled");
}

#[tokio::test]
async fn restart_recovery_owner_resume_retires_only_the_interrupted_prompt() {
    use crate::durable_state::agent_lifecycle::{ExecutionState, Operation, Outcome};
    let mut config = DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let (runtime, room, agent, prompt) = runtime_with_active_prompt_config(
        crate::session::DurablePromptDeliveryPhase::Delivered,
        std::env::current_dir().unwrap().to_str().unwrap(),
        config,
    );
    let Outcome::Task(task) = runtime
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::Begin {
            owner: "local".into(),
            room: room.clone(),
            agent: agent.clone(),
            prompt: prompt.clone(),
            run: None,
            now: 1,
        })
        .unwrap()
    else {
        panic!("task should begin")
    };
    let Outcome::Task(blocked) = runtime
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::Block {
            task: task.task_id.clone(),
            prompt: prompt.clone(),
            reason: "Owner approval required".into(),
        })
        .unwrap()
    else {
        panic!("task should block")
    };
    let run = runtime
        .owned
        .provider_store
        .get_run_for_agent(&room, &agent)
        .unwrap();
    runtime
        .owned
        .provider_store
        .mark_run_ended_provider_only(&room, run.id())
        .unwrap();
    let summary = runtime.recover_durable_runtime_after_restart().await;
    assert_eq!(summary.blocked_local_prompts_preserved, 1);
    assert_eq!(summary.failed_reconciliations, 0);
    assert!(runtime
        .owned
        .provider_store
        .get_run_for_agent(&room, &agent)
        .is_none());
    let id = format!(
        "task-blocked-{}-{}",
        blocked.task_id, blocked.blocked_revision
    );
    runtime
        .resolve_terminal_runtime_interaction(&room, &id, "resume", None, Some("local"))
        .await
        .unwrap();
    for _ in 0..200 {
        let next = runtime
            .owned
            .durable_state_store
            .agent_tasks(Some(&room), Some(&agent))
            .unwrap()
            .remove(0);
        if next.prompt_id != prompt {
            assert_eq!(next.task_id, task.task_id);
            assert_ne!(next.state, ExecutionState::Blocked, "{}", next.reason);
            assert_ne!(next.state, ExecutionState::Cancelled);
            let session = runtime.owned.session_store.get_session(&room).unwrap();
            assert!(runtime
                .owned
                .prompt_state_owner
                .active_prompt_for_agent(&session, &agent)
                .is_none_or(|active| active.id() != prompt));
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("explicit Resume must start its continuation, not queue behind the interrupted prompt");
}

#[tokio::test]
async fn restart_recovery_paused_goal_can_resume_without_its_old_executor() {
    use crate::durable_state::agent_lifecycle::{ExecutionState, Operation, Outcome};
    let mut config = DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let (runtime, room, agent, prompt) = runtime_with_active_prompt_config(
        crate::session::DurablePromptDeliveryPhase::Delivered,
        std::env::current_dir().unwrap().to_str().unwrap(),
        config,
    );
    let Outcome::Task(task) = runtime
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::Begin {
            owner: "local".into(),
            room: room.clone(),
            agent: agent.clone(),
            prompt: prompt.clone(),
            run: None,
            now: 1,
        })
        .unwrap()
    else {
        panic!("task should begin")
    };
    let Outcome::Task(goal) = runtime
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::StartGoal {
            task: task.task_id.clone(),
            owner: "local".into(),
            objective: "Complete approved work".into(),
            minutes: 10,
            now: 2,
        })
        .unwrap()
    else {
        panic!("goal should start")
    };
    let Outcome::Task(paused) = runtime
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::ControlGoal {
            task: task.task_id.clone(),
            owner: "local".into(),
            revision: goal.revision,
            action: "pause".into(),
            value: None,
            now: 3,
        })
        .unwrap()
    else {
        panic!("goal should pause")
    };
    let run = runtime
        .owned
        .provider_store
        .get_run_for_agent(&room, &agent)
        .unwrap();
    runtime
        .owned
        .provider_store
        .mark_run_ended_provider_only(&room, run.id())
        .unwrap();
    let summary = runtime.recover_durable_runtime_after_restart().await;
    assert_eq!(summary.blocked_local_prompts_preserved, 1);
    assert_eq!(summary.failed_reconciliations, 0);
    let attachment = runtime
        .owned
        .attach(AttachRequest::for_user(
            &room,
            "owner-after-restart",
            ClientCapabilityLevel::FullTerminal,
            "local",
        ))
        .unwrap();
    runtime
        .control_goal_prompt(
            &crate::local::SubmitPromptRequest {
                session_id: room.clone(),
                attachment_id: attachment.id().into(),
                target_agent_id: Some(agent.clone()),
                prompt: format!(
                    "/goal resume {} {}",
                    paused.goal.as_ref().unwrap().id,
                    paused.revision
                ),
                attachments: vec![],
            },
            "local",
        )
        .await
        .unwrap()
        .expect("goal control should handle resume");
    let next = runtime
        .owned
        .durable_state_store
        .agent_tasks(Some(&room), Some(&agent))
        .unwrap()
        .remove(0);
    assert_eq!(next.task_id, task.task_id);
    assert_eq!(next.state, ExecutionState::Working);
    assert_ne!(next.prompt_id, prompt);
    assert!(!next.goal.unwrap().paused);
}

#[tokio::test]
async fn restart_recovery_released_human_control_wait_has_no_stale_executor() {
    use crate::durable_state::agent_lifecycle::{ExecutionState, Operation, Outcome};
    let mut config = DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let (runtime, room, agent, prompt) = runtime_with_active_prompt_config(
        crate::session::DurablePromptDeliveryPhase::Delivered,
        std::env::current_dir().unwrap().to_str().unwrap(),
        config,
    );
    let Outcome::Task(task) = runtime
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::Begin {
            owner: "local".into(),
            room: room.clone(),
            agent: agent.clone(),
            prompt: prompt.clone(),
            run: None,
            now: 1,
        })
        .unwrap()
    else {
        panic!("task should begin")
    };
    runtime
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::Block {
            task: task.task_id.clone(),
            prompt: prompt.clone(),
            reason: "Computer environment_input_owned; human must release the desktop".into(),
        })
        .unwrap();
    let run = runtime
        .owned
        .provider_store
        .get_run_for_agent(&room, &agent)
        .unwrap();
    runtime
        .owned
        .provider_store
        .mark_run_ended_provider_only(&room, run.id())
        .unwrap();
    // The concrete ownership refusal and released Computer already authorize a
    // continuation. A retained prompt without an executor cannot suppress it.
    let summary = runtime.recover_durable_runtime_after_restart().await;
    assert_eq!(summary.failed_reconciliations, 0);
    let next = runtime
        .owned
        .durable_state_store
        .agent_tasks(Some(&room), Some(&agent))
        .unwrap()
        .remove(0);
    assert_eq!(next.task_id, task.task_id);
    assert_ne!(next.state, ExecutionState::Blocked, "{}", next.reason);
    assert_ne!(next.state, ExecutionState::Cancelled);
    assert_ne!(next.prompt_id, prompt);
}
