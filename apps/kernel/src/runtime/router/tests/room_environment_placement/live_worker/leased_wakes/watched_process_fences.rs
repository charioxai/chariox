//! MP-08 / MP-09 / MP-10 / MP-11: home consent does not change worker run modes.
use super::*;
use crate::durable_state::agent_lifecycle::{AgentWake, LeasedProcessBinding};
use crate::provider::{AgentExecutionMode, AgentPermissionLevel, ProviderWriteAccessMode};
use crate::transport::relay_peer::LeasedWatchedProcessAction;

#[test]
fn mp10_a10_sudo_watched_process_keeps_worker_mode_and_write_fences() {
    run_test(sudo_watched_process_keeps_worker_mode_and_write_fences);
}

async fn sudo_watched_process_keeps_worker_mode_and_write_fences() {
    let _tools = with_room_tools();
    let mut fixture = LiveWorker::start_with_home_passkey(OWNER_PASSKEY).await;
    let worker = start_agent_worker(&fixture, agent_worker_state(&fixture)).await;
    let room = fixture.rooms[0].clone();
    let attachment = owner_attachment(&fixture, &room).await;
    let agent =
        spawn_on_agent_worker_with_model(&mut fixture, &room, "controlled-cancel-idle").await;
    let (owner, _) = open_leased_sudo(
        &fixture,
        &room,
        &attachment,
        &agent,
        Duration::from_secs(120),
    )
    .await;
    let prompt = fixture
        .home
        .runtime_state
        .list_sudo_turns(&owner)
        .pop()
        .unwrap()
        .prompt_id
        .unwrap();
    let run = worker.run_for(&fixture, &agent, &prompt).await;
    let home_store = store(&fixture).await;
    let task = home_store
        .agent_tasks(Some(&room), Some(&agent))
        .unwrap()
        .into_iter()
        .find(|task| task.prompt_id == prompt)
        .unwrap();
    let remote = fixture
        .home
        .app
        .lock()
        .await
        .agents()
        .get_agent(&agent)
        .unwrap()
        .remote_execution()
        .unwrap()
        .clone();
    let binding = LeasedProcessBinding {
        worker_kernel_id: remote.worker_kernel_id.clone(),
        worker_machine_id: remote.worker_machine_id,
        execution_lease_id: remote.execution_lease_id,
        leased_agent_id: remote.leased_agent_id.clone(),
        home_prompt_id: prompt.clone(),
        worker_provider_run_id: run.id().into(),
    };
    let mut allowed = Vec::new();
    let mut replies = Vec::new();
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
        let mut snapshot = serde_json::to_value(&run).unwrap();
        snapshot["write_access_mode"] = serde_json::to_value(write_access).unwrap();
        let restricted = serde_json::from_value(snapshot).unwrap();
        worker
            .router
            .app
            .lock()
            .await
            .providers_mut()
            .insert_run_for_test(restricted);
        worker
            .router
            .app
            .lock()
            .await
            .providers()
            .update_run_execution_config(run.id(), mode, AgentPermissionLevel::Required)
            .unwrap();
        let id = format!("restricted-{:032x}", rand::random::<u128>());
        let wake = AgentWake {
            registration_id: format!("completion-{id}"),
            id,
            task_id: task.task_id.clone(),
            room_id: room.clone(),
            agent_id: agent.clone(),
            kind: "process".into(),
            label: "MP-10 restricted worker process".into(),
            state: String::new(),
            created_at_ms: crate::session::unix_epoch_ms(),
            verified_at_ms: None,
            next_due_ms: None,
            interval_ms: None,
            command: vec!["/bin/true".into()],
            match_text: None,
            matched_at_ms: None,
            pid: None,
            exit_code: None,
            summary: String::new(),
            detail: json!(null),
            fire_count: 0,
            missed_fires: 0,
            last_fired_at_ms: None,
            last_sequence: None,
            last_delivery: None,
            last_delivered_at_ms: None,
            last_acknowledged_at_ms: None,
            alerted_sequence: None,
        };
        let result = timeout(
            Duration::from_secs(10),
            send_peer_request_via_temporary_connection(
                &fixture.home_state.config,
                ClientTarget {
                    daemon_id: Some(remote.worker_kernel_id.clone()),
                    daemon_alias: None,
                },
                RelayPeerRequest::LeasedWatchedProcess {
                    leased_agent_id: remote.leased_agent_id.clone(),
                    action: LeasedWatchedProcessAction::Start {
                        wake,
                        binding: binding.clone(),
                        cwd: None,
                    },
                },
            ),
        )
        .await
        .expect("restricted worker admission must settle");
        if result.is_ok() {
            allowed.push(format!("{mode:?}/{write_access:?}"));
        }
        replies.push(format!("{mode:?}/{write_access:?}: {result:?}"));
    }
    let receipts = worker
        .router
        .app
        .lock()
        .await
        .durable_state_store()
        .leased_process_receipts()
        .unwrap();
    worker.stop().await;
    fixture.stop().await;
    assert!(
        allowed.is_empty(),
        "sudo bypassed worker fences: {allowed:?}"
    );
    assert!(
        receipts.is_empty(),
        "restricted calls published worker process intents: {replies:?}"
    );
    assert!(
        replies
            .iter()
            .all(|reply| reply.contains("unrestricted build run")),
        "each restriction must be refused by the worker mode gate: {replies:?}"
    );
}
