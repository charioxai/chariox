use super::*;
use crate::local::*;
use crate::runtime::command::KernelCommand;

fn fixture(
    automatic: bool,
) -> (
    KernelRuntimeState,
    KernelCommand,
    ControlPlaneRequest,
    MachineAllocationCapability,
    crate::test_support::TestWorktree,
) {
    let workspace = crate::test_support::TestWorktree::new("offload-owner-decision");
    let mut config = crate::config::DaemonConfig::for_tests();
    config.user_config.agents.offload_without_asking = Some(automatic);
    let mut app = crate::DaemonApp::bootstrap(config).unwrap();
    let (room, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(workspace.session_request())
        .unwrap();
    let request = ControlPlaneRequest::AcquireJob {
        capability_revision: "v1".into(),
        request_id: "request".into(),
        job_ref: "job".into(),
        needs: JobNeeds::default(),
        timeout_s: 60,
    };
    let mut command = KernelCommand::from_local_request(
        "acquire-job",
        None,
        None,
        &LocalDaemonRequest::ControlPlane(request.clone()),
    );
    command.agent_id = Some(agent.id().into());
    command.session_id = Some(room.id().into());
    command.caller.connection_class = Some(KernelConnectionClass::KernelAgent);
    let caps = MachineAllocationCapability {
        revision: "v1".into(),
        actions: vec!["acquire_job".into()],
        offer: None,
        job_default_class_id: Some("build".into()),
        job_offers: vec![JobMachineOffer {
            class_id: "build".into(),
            vcpu: 4,
            usable_memory_mib: 7000,
            disk_gib: 40,
            rate_per_minute: JobMachineRate {
                amount_micros: 600,
                currency: "USD".into(),
            },
        }],
    };
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
        Arc::new(tokio::sync::Mutex::new(app)),
        1,
    );
    (
        router.runtime_state().with_job_router(router),
        command,
        request,
        caps,
        workspace,
    )
}

#[tokio::test]
async fn offload_automatic_setting_off_awaits_one_human_decision_before_returning() {
    let (state, command, request, caps, _workspace) = fixture(false);
    let gate_state = state.clone();
    let room = command.session_id.clone().unwrap();
    let gate = tokio::spawn(async move {
        gate_state
            .approve_machine_acquisition(&command, &request, Some(&caps))
            .await
    });
    let id = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let snapshot = state.owned.session_store.get_session(&room).unwrap();
            if let Some(card) = snapshot.active_interactions().first() {
                assert!(card.agent_id().is_none());
                assert!(card.related_agent_id().is_some());
                assert!(card.shown_in_agent_id().is_some());
                assert_eq!(snapshot.active_interactions().len(), 1);
                assert!(card.message().contains("USD per minute"));
                break card.id().to_owned();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        !gate.is_finished(),
        "no allocation caller can continue before the owner answers"
    );
    let answer = RespondToInteractionRequest {
        session_id: room.clone(),
        interaction_id: id.clone(),
        choice_id: "approve".into(),
        custom_reply: None,
        passkey: None,
        passkey_remember_minutes: None,
    };
    assert!(
        state.interaction_requires_terminal(&answer),
        "an agent cannot approve its own rental"
    );
    let owner = state
        .owned
        .session_store
        .get_session(&room)
        .unwrap()
        .owner_user_id()
        .to_owned();
    state
        .resolve_terminal_runtime_interaction(&room, &id, "deny", None, Some(&owner))
        .await
        .unwrap();
    let error = gate.await.unwrap().unwrap_err().to_string();
    assert!(error.contains("denied"));
    assert!(error.contains("nothing was allocated or charged"));
}

#[tokio::test]
async fn offload_automatic_setting_on_has_no_decision_and_off_applies_to_next_request() {
    let (state, command, request, caps, _workspace) = fixture(true);
    state
        .approve_machine_acquisition(&command, &request, Some(&caps))
        .await
        .unwrap();
    let room = command.session_id.as_deref().unwrap();
    assert!(state
        .owned
        .session_store
        .get_session(room)
        .unwrap()
        .active_interactions()
        .is_empty());
}

#[test]
fn offload_setting_persists_and_unset_restores_off() {
    let mut config = crate::config::DaemonConfig::for_tests();
    assert_ne!(config.user_config.agents.offload_without_asking, Some(true));
    config
        .set_user_config_value("agents.offload_without_asking", "true")
        .unwrap();
    let persisted = std::fs::read(config.user_config_path()).unwrap();
    let record: crate::config::CharioxUserConfig =
        toml::from_str(std::str::from_utf8(&persisted).unwrap()).unwrap();
    assert_eq!(record.agents.offload_without_asking, Some(true));
    config
        .unset_user_config_value("agents.offload_without_asking")
        .unwrap();
    assert_ne!(config.user_config.agents.offload_without_asking, Some(true));
}
