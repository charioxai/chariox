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
        offers: vec![],
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
        !state.interaction_person_only(&answer),
        "authorized parent agents may answer existing offload approvals"
    );
    let owner = state
        .owned
        .session_store
        .get_session(&room)
        .unwrap()
        .owner_user_id()
        .to_owned();
    let agent_answer = state
        .answer_terminal_runtime_interaction(
            &room,
            &id,
            "approve",
            None,
            Some(&owner),
            None,
            None,
            Some(KernelConnectionClass::KernelAgent),
        )
        .await;
    assert!(
        agent_answer.is_err(),
        "the agent cannot approve its own request"
    );
    assert!(!gate.is_finished());
    state
        .resolve_terminal_runtime_interaction(&room, &id, "deny", None, Some(&owner))
        .await
        .unwrap();
    let error = gate.await.unwrap().unwrap_err().to_string();
    assert!(error.contains("Denied by"));
    assert!(error.contains("The request was not sent to the service"));
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
    let mut config = state.owned.config_projection.snapshot();
    config.user_config.agents.offload_without_asking = Some(false);
    state.owned.config_projection.update(config);
    let gate_state = state.clone();
    let room = room.to_owned();
    let gate = tokio::spawn(async move {
        gate_state
            .approve_machine_acquisition(&command, &request, Some(&caps))
            .await
    });
    let id = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let snapshot = state.owned.session_store.get_session(&room).unwrap();
            if let Some(card) = snapshot.active_interactions().first() {
                break card.id().to_owned();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(!gate.is_finished());
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
    assert!(gate
        .await
        .unwrap()
        .unwrap_err()
        .to_string()
        .contains("Denied by"));
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

#[tokio::test]
async fn offload_manual_machine_request_uses_the_same_owner_gate_and_service_minute_rate() {
    let (state, command, _, mut caps, _workspace) = fixture(false);
    caps.offer = Some(
        serde_json::from_value(serde_json::json!({
            "machine_type":"Standard", "vcpu":4, "ram_gb":8.0, "disk_gb":40,
            "location":"Region", "terms":{"amount":"0.036","currency":"USD","period":"hour"},
            "rate_per_minute":{"amount_micros":600,"currency":"USD"}
        }))
        .unwrap(),
    );
    let request = ControlPlaneRequest::Acquire {
        reason: None,
        capability_revision: "v1".into(),
        request_id: "manual".into(),
        class_id: None,
        region: None,
    };
    let room = command.session_id.clone().unwrap();
    let gate_state = state.clone();
    let gate = tokio::spawn(async move {
        gate_state
            .approve_machine_acquisition(&command, &request, Some(&caps))
            .await
    });
    let id = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let snapshot = state.owned.session_store.get_session(&room).unwrap();
            if let Some(card) = snapshot.active_interactions().first() {
                assert_eq!(snapshot.active_interactions().len(), 1);
                assert!(card.message().contains("0.0006 USD per minute"));
                assert!(card.message().contains("total depends"));
                break card.id().to_owned();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(!gate.is_finished());
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
    assert!(gate
        .await
        .unwrap()
        .unwrap_err()
        .to_string()
        .contains("The request was not sent to the service"));
}

pub(super) fn option(class_id: &str, region: &str, available: bool) -> MachineOption {
    MachineOption {
        class_id: class_id.into(),
        region: region.into(),
        vcpu: 2,
        ram_gb: 4.0,
        disk_gb: 51,
        rate_per_minute: JobMachineRate {
            amount_micros: 284,
            currency: "EUR".into(),
        },
        available,
        note: None,
    }
}

#[test]
fn the_machine_tools_list_options_and_acquire_takes_the_chosen_class() {
    let specs = super::machine_tool_specs();
    let names: Vec<_> = specs.iter().map(|spec| spec.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "chariox.machines.options",
            "chariox.machines.acquire",
            "chariox.machines.status",
            "chariox.machines.release"
        ]
    );
    let acquire = specs
        .iter()
        .find(|spec| spec.name == "chariox.machines.acquire")
        .unwrap();
    for field in ["request_id", "class_id", "region", "reason"] {
        assert!(
            acquire.input_schema["properties"][field].is_object(),
            "{field}"
        );
    }
    assert_eq!(
        acquire.input_schema["required"],
        serde_json::json!(["reason"])
    );
    assert_eq!(
        acquire.input_schema["additionalProperties"],
        serde_json::json!(false)
    );
}

#[tokio::test]
async fn the_owner_is_asked_about_the_option_the_agent_chose() {
    let (state, command, _, mut caps, _workspace) = fixture(false);
    caps.offers = vec![option("agent-vm-s", "auto", true)];
    let request = ControlPlaneRequest::Acquire {
        reason: None,
        capability_revision: "v1".into(),
        request_id: "chosen".into(),
        class_id: Some("agent-vm-s".into()),
        region: Some("auto".into()),
    };
    let room = command.session_id.clone().unwrap();
    let gate_state = state.clone();
    let gate = tokio::spawn(async move {
        gate_state
            .approve_machine_acquisition(&command, &request, Some(&caps))
            .await
    });
    let message = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let snapshot = state.owned.session_store.get_session(&room).unwrap();
            if let Some(card) = snapshot.active_interactions().first() {
                break card.message().to_owned();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(message.contains("agent-vm-s in auto"), "{message}");
    assert!(message.contains("EUR per minute"), "{message}");
    gate.abort();
}

#[tokio::test]
async fn the_owner_is_asked_about_a_named_option_even_when_it_is_no_longer_listed() {
    // A retry of an accepted request must reach the service whatever the list says now.
    let (state, command, _, caps, _workspace) = fixture(false);
    let request = ControlPlaneRequest::Acquire {
        reason: None,
        capability_revision: "v1".into(),
        request_id: "retry".into(),
        class_id: Some("gone".into()),
        region: Some("auto".into()),
    };
    let room = command.session_id.clone().unwrap();
    let gate_state = state.clone();
    let gate = tokio::spawn(async move {
        gate_state
            .approve_machine_acquisition(&command, &request, Some(&caps))
            .await
    });
    let message = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let snapshot = state.owned.session_store.get_session(&room).unwrap();
            if let Some(card) = snapshot.active_interactions().first() {
                break card.message().to_owned();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(message.contains("Acquire gone in auto"), "{message}");
    gate.abort();
}

#[test]
fn the_service_request_id_is_scoped_to_the_kernel_and_agent() {
    let id = super::choice::service_request_id("kernel", "agent", "R1");
    assert!(id.len() <= 128, "{id}");
    assert!(id
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'));
    assert_eq!(
        id,
        super::choice::service_request_id("kernel", "agent", "R1")
    );
    assert_ne!(
        id,
        super::choice::service_request_id("kernel", "other", "R1")
    );
    assert_ne!(
        id,
        super::choice::service_request_id("other", "agent", "R1")
    );
    assert_ne!(
        id,
        super::choice::service_request_id("kernel", "agent", "r1")
    );
}
