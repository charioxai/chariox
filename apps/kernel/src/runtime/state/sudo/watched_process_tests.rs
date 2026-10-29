//! MP-08 / MP-09 / MP-10 / MP-11: sudo never changes execution or write mode.
use super::tests::{fixture_with_options, running};

#[tokio::test]
async fn sudo_operations_watched_commands_keep_plan_and_write_fences() {
    use crate::durable_state::agent_lifecycle::{Operation, Outcome};
    use crate::provider::{AgentExecutionMode, AgentPermissionLevel};
    let mut failures = Vec::new();
    for (mode, write_access) in [
        (
            AgentExecutionMode::Plan,
            crate::provider::ProviderWriteAccessMode::Unrestricted,
        ),
        (
            AgentExecutionMode::Build,
            crate::provider::ProviderWriteAccessMode::WorkspaceLiveSyncManaged,
        ),
        (
            AgentExecutionMode::Build,
            crate::provider::ProviderWriteAccessMode::WorkspaceLiveSyncTracked,
        ),
    ] {
        let f = fixture_with_options(None, true);
        let turn = running(&f);
        let mut snapshot = serde_json::to_value(&f.run).unwrap();
        snapshot["write_access_mode"] = serde_json::to_value(write_access).unwrap();
        let run: crate::provider::RuntimeProviderRun = serde_json::from_value(snapshot).unwrap();
        f.app.lock().await.providers_mut().insert_run_for_test(run);
        f.state
            .owned
            .provider_store
            .update_run_execution_config(f.run.id(), mode, AgentPermissionLevel::Required)
            .unwrap();
        let Outcome::Task(task) = f
            .state
            .owned
            .durable_state_store
            .agent_lifecycle(Operation::Begin {
                owner: turn.owner_user_id.clone(),
                room: turn.session_id.clone(),
                agent: turn.agent_id.clone(),
                prompt: turn.prompt_id.clone().unwrap(),
                run: Some(f.run.id().into()),
                now: crate::session::unix_epoch_ms(),
            })
            .unwrap()
        else {
            panic!("task missing")
        };
        let result = f
            .router
            .dispatch_authenticated_runtime_tool_call(
                "sudo-fixture-bearer",
                "chariox.events.process",
                serde_json::json!({"task_id":task.task_id, "origin_prompt_id":turn.prompt_id,
                "label":"restricted run must not execute", "argv":["/bin/echo","must-not-run"],
                "match_text":"must-not-run"}),
            )
            .await;
        f.state.owned.agent_wakes.processes.shutdown();
        if result.is_ok() {
            failures.push(format!("{mode:?}/{write_access:?}"));
        }
    }
    assert!(failures.is_empty(), "sudo bypassed fences: {failures:?}");
}

#[tokio::test]
async fn watched_process_rechecks_modes_after_owner_approval() {
    use crate::durable_state::agent_lifecycle::{Operation, Outcome};
    use crate::provider::{AgentExecutionMode, AgentPermissionLevel, ProviderWriteAccessMode};
    let mut failures = Vec::new();
    for (mode, write_access) in [
        (
            AgentExecutionMode::Plan,
            ProviderWriteAccessMode::Unrestricted,
        ),
        (
            AgentExecutionMode::Build,
            ProviderWriteAccessMode::WorkspaceLiveSyncManaged,
        ),
        (
            AgentExecutionMode::Build,
            ProviderWriteAccessMode::WorkspaceLiveSyncTracked,
        ),
    ] {
        let f = super::tests::fixture_with_run_profile(None, true, "claude", "claude");
        let turn = running(&f);
        f.state.owned.sudo_turns.lock().unwrap().clear();
        f.state
            .owned
            .provider_store
            .update_run_execution_config(
                f.run.id(),
                AgentExecutionMode::Build,
                AgentPermissionLevel::Yolo,
            )
            .unwrap();
        let Outcome::Task(task) = f
            .state
            .owned
            .durable_state_store
            .agent_lifecycle(Operation::Begin {
                owner: turn.owner_user_id.clone(),
                room: turn.session_id.clone(),
                agent: turn.agent_id.clone(),
                prompt: turn.prompt_id.clone().unwrap(),
                run: Some(f.run.id().into()),
                now: crate::session::unix_epoch_ms(),
            })
            .unwrap()
        else {
            panic!("task missing")
        };
        let request = serde_json::json!({"task_id":task.task_id, "origin_prompt_id":turn.prompt_id,
            "label":"mode changes while command approval is open", "argv":["/bin/true"]});
        let (result, ()) = tokio::join!(
            f.router.dispatch_authenticated_runtime_tool_call(
                "sudo-fixture-bearer",
                "chariox.events.process",
                request
            ),
            async {
                let interaction = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                    loop {
                        if let Some(interaction) = f
                            .state
                            .owned
                            .session_snapshot(&turn.session_id)
                            .unwrap()
                            .active_interactions()
                            .iter()
                            .find(|interaction| interaction.id().starts_with("watched-process-"))
                            .cloned()
                        {
                            break interaction;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                    }
                })
                .await
                .expect("native watched command must ask for owner approval");
                let mut snapshot = serde_json::to_value(&f.run).unwrap();
                snapshot["write_access_mode"] = serde_json::to_value(write_access).unwrap();
                f.app
                    .lock()
                    .await
                    .providers_mut()
                    .insert_run_for_test(serde_json::from_value(snapshot).unwrap());
                f.state
                    .owned
                    .provider_store
                    .update_run_execution_config(f.run.id(), mode, AgentPermissionLevel::Yolo)
                    .unwrap();
                f.state
                    .resolve_runtime_interaction(
                        &turn.session_id,
                        interaction.id(),
                        "allow_once",
                        None,
                    )
                    .await
                    .unwrap();
            }
        );
        f.state.owned.agent_wakes.processes.shutdown();
        if result.is_ok() {
            failures.push(format!("{mode:?}/{write_access:?}"));
        }
    }
    assert!(
        failures.is_empty(),
        "stale approval bypassed current run fences: {failures:?}"
    );
}
