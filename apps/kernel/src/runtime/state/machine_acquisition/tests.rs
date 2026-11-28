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
        "an external agent approves a rental for the person"
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
    assert!(error.contains("denied"));
    assert!(error.contains("it was not sent to the service"));
}

#[tokio::test]
async fn machine_acquisition_reclaims_stale_owner_decisions_before_admission() {
    let (state, command, request, caps, _workspace) = fixture(false);
    let room = command.session_id.clone().unwrap();
    let owner = state
        .owned
        .session_store
        .get_session(&room)
        .unwrap()
        .owner_user_id()
        .to_owned();
    let mut expired = Vec::new();
    let mut abandoned = Vec::new();
    for index in 0..8 {
        let id = format!("{room}-stale-{index}");
        let interaction = RuntimeInteraction::for_kernel_operation(
            &id,
            &id,
            "Unrelated action",
            "Review this action",
            vec![RuntimeInteractionChoice::new("deny", "Deny", "deny", None)],
        )
        .with_timeout_sec(300);
        let receiver = state
            .create_kernel_operation_interaction(
                crate::runtime::state::CardKind::OwnerControl,
                "",
                &owner,
                interaction,
            )
            .await
            .unwrap();
        if index < 4 {
            expired.push((id, receiver));
        } else {
            abandoned.push(receiver);
        }
    }
    for (id, _) in &expired {
        state
            .owned
            .pending_interactions
            .write()
            .get_mut(id)
            .unwrap()
            .kernel_operation_deadline = Some(std::time::Instant::now());
    }
    drop(abandoned);

    let gate_state = state.clone();
    let mut gate = tokio::spawn(async move {
        gate_state
            .approve_machine_acquisition(&command, &request, Some(&caps))
            .await
    });
    let card = tokio::select! {
        result = &mut gate => panic!("machine approval ended without a card: {result:?}"),
        card = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let session = state.owned.session_store.get_session(&room).unwrap();
                if let Some(card) = session.active_interactions().first() {
                    break card.id().to_owned();
                }
                tokio::task::yield_now().await;
            }
        }) => card.expect("machine approval card is missing"),
    };
    assert_eq!(state.list_owner_interactions(&owner).len(), 1);
    for (_, mut receiver) in expired {
        assert_eq!(receiver.try_recv().unwrap().status, "timed_out");
    }
    state
        .resolve_terminal_runtime_interaction(&room, &card, "deny", None, Some(&owner))
        .await
        .unwrap();
    assert!(gate
        .await
        .unwrap()
        .unwrap_err()
        .to_string()
        .contains("denied"));
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
    // A retry of the request admitted while the setting was on continues it; Off applies from the next request.
    let next = ControlPlaneRequest::AcquireJob {
        capability_revision: "v1".into(),
        request_id: "next-request".into(),
        job_ref: "next-job".into(),
        needs: JobNeeds::default(),
        timeout_s: 60,
    };
    let gate_state = state.clone();
    let room = room.to_owned();
    let gate = tokio::spawn(async move {
        gate_state
            .approve_machine_acquisition(&command, &next, Some(&caps))
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
        .contains("denied"));
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
        access_scope: None,
        capability_revision: "v1".into(),
        request_id: "manual".into(),
        class_id: None,
        region: None,
        timeout_s: None,
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
        .contains("it was not sent to the service"));
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
    for field in ["request_id", "class_id", "region"] {
        assert!(
            acquire.input_schema["properties"][field].is_object(),
            "{field}"
        );
    }
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
        access_scope: None,
        capability_revision: "v1".into(),
        request_id: "chosen".into(),
        class_id: Some("agent-vm-s".into()),
        region: Some("auto".into()),
        timeout_s: None,
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
        access_scope: None,
        capability_revision: "v1".into(),
        request_id: "retry".into(),
        class_id: Some("gone".into()),
        region: Some("auto".into()),
        timeout_s: None,
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

/// The card of an approval a try is waiting on, once the kernel raised it.
async fn waiting_card(state: &KernelRuntimeState, room: &str) -> String {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let snapshot = state.owned.session_store.get_session(room).unwrap();
            if let Some(card) = snapshot.active_interactions().first() {
                break card.id().to_owned();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the owner's card is raised")
}

fn owner_of(state: &KernelRuntimeState, room: &str) -> String {
    state
        .owned
        .session_store
        .get_session(room)
        .unwrap()
        .owner_user_id()
        .to_owned()
}

/// The receipts of the Room, as a client reads them.
fn receipts(state: &KernelRuntimeState, room: &str) -> Vec<crate::session::MachineApprovalReceipt> {
    state
        .owned
        .session_store
        .get_session(room)
        .unwrap()
        .machine_approval_receipts()
        .to_vec()
}

/// Runs one try of `request` and answers its card with `choice`.
async fn approve_with(
    state: &KernelRuntimeState,
    command: &KernelCommand,
    request: &ControlPlaneRequest,
    caps: &MachineAllocationCapability,
    choice: &str,
) -> Result<Approval, DaemonError> {
    let room = command.session_id.clone().unwrap();
    let gate = {
        let (state, command, request, caps) = (
            state.clone(),
            command.clone(),
            request.clone(),
            caps.clone(),
        );
        tokio::spawn(async move {
            state
                .approve_machine_acquisition(&command, &request, Some(&caps))
                .await
        })
    };
    let id = waiting_card(state, &room).await;
    state
        .resolve_terminal_runtime_interaction(
            &room,
            &id,
            choice,
            None,
            Some(&owner_of(state, &room)),
        )
        .await
        .unwrap();
    gate.await.unwrap()
}

#[tokio::test]
async fn an_approved_request_stays_visible_as_a_receipt_until_the_service_answers() {
    let (state, command, request, caps, _workspace) = fixture(false);
    let room = command.session_id.clone().unwrap();
    assert!(receipts(&state, &room).is_empty());
    let approval = approve_with(&state, &command, &request, &caps, "approve")
        .await
        .unwrap();
    assert!(matches!(approval, Approval::Owner), "{approval:?}");
    // The card is gone and the request has not reached the service yet.
    assert!(state
        .owned
        .session_store
        .get_session(&room)
        .unwrap()
        .active_interactions()
        .is_empty());
    let held = receipts(&state, &room);
    assert_eq!(held.len(), 1);
    assert_eq!(
        held[0].state,
        crate::session::MachineApprovalReceiptState::Approving
    );
    assert!(held[0].id.starts_with("machine-acquisition-"));
    assert!(held[0].shown_in_agent_id.is_some());
    assert!(
        held[0].message.contains("USD per minute"),
        "{}",
        held[0].message
    );
    state.machine_approval_sent(&command, &request);
    assert_eq!(
        receipts(&state, &room)[0].state,
        crate::session::MachineApprovalReceiptState::Sent
    );
}

#[tokio::test(start_paused = true)]
async fn an_approval_nobody_answers_leaves_an_expired_receipt() {
    let (state, command, request, caps, _workspace) = fixture(false);
    let room = command.session_id.clone().unwrap();
    let gate = {
        let (state, command, request, caps) = (
            state.clone(),
            command.clone(),
            request.clone(),
            caps.clone(),
        );
        tokio::spawn(async move {
            state
                .approve_machine_acquisition(&command, &request, Some(&caps))
                .await
        })
    };
    waiting_card(&state, &room).await;
    tokio::time::sleep(Duration::from_secs(301)).await;
    let error = gate.await.unwrap().unwrap_err().to_string();
    assert!(error.contains("nobody answered"), "{error}");
    let expired = receipts(&state, &room);
    assert_eq!(expired.len(), 1);
    assert_eq!(
        expired[0].state,
        crate::session::MachineApprovalReceiptState::Expired
    );
    assert!(expired[0].expires_at_ms.is_some());
    // A denial leaves no receipt: the person said no.
    let denied = {
        let (state, command, mut request) = (state.clone(), command.clone(), request.clone());
        if let ControlPlaneRequest::AcquireJob { request_id, .. } = &mut request {
            *request_id = "request-2".into();
        }
        (state, command, request)
    };
    let denial = approve_with(&denied.0, &denied.1, &denied.2, &caps, "deny").await;
    assert!(denial.unwrap_err().to_string().contains("denied"));
    assert_eq!(receipts(&state, &room).len(), 1);
}

#[tokio::test]
async fn an_approved_request_that_was_sent_replays_at_its_approved_revision_when_the_quote_changes()
{
    let (state, command, request, caps, _workspace) = fixture(false);
    let room = command.session_id.clone().unwrap();
    // Approved and sent; the service may have accepted it and lost the response.
    let first = approve_with(&state, &command, &request, &caps, "approve")
        .await
        .unwrap();
    assert!(matches!(first, Approval::Owner), "{first:?}");
    state.machine_approval_sent(&command, &request);

    // The rate and revision change before the retry: another quote, not another request.
    let mut dearer = caps.clone();
    dearer.revision = "v2".into();
    dearer.job_offers[0].rate_per_minute.amount_micros = 900;
    let retry = request.with_capability_revision("v2");
    let replay = tokio::time::timeout(
        Duration::from_secs(2),
        state.approve_machine_acquisition(&command, &retry, Some(&dearer)),
    )
    .await
    .expect("the owner is not asked again for a request already sent")
    .unwrap();
    assert!(
        matches!(&replay, Approval::Replay { capability_revision } if capability_revision == "v1"),
        "{replay:?}"
    );
    assert!(state
        .owned
        .session_store
        .get_session(&room)
        .unwrap()
        .active_interactions()
        .is_empty());
    // At its own revision it is a plain retry; the replayed request carries the approved one.
    let plain = state
        .approve_machine_acquisition(&command, &request, Some(&caps))
        .await
        .unwrap();
    assert!(matches!(plain, Approval::Owner), "{plain:?}");
    assert_eq!(request.capability_revision(), Some("v1"));
    assert_eq!(retry.with_capability_revision("v1"), request);
}

#[tokio::test]
async fn a_replay_belongs_to_one_request_a_different_request_asks_the_owner() {
    let (state, command, request, caps, _workspace) = fixture(false);
    let room = command.session_id.clone().unwrap();
    approve_with(&state, &command, &request, &caps, "approve")
        .await
        .unwrap();
    let mut other = request.clone();
    if let ControlPlaneRequest::AcquireJob { request_id, .. } = &mut other {
        *request_id = "another-request".into();
    }
    let gate = {
        let (state, command, other, caps) =
            (state.clone(), command.clone(), other.clone(), caps.clone());
        tokio::spawn(async move {
            state
                .approve_machine_acquisition(&command, &other, Some(&caps))
                .await
        })
    };
    let id = waiting_card(&state, &room).await;
    assert!(
        !gate.is_finished(),
        "another request waits for its own decision"
    );
    state
        .resolve_terminal_runtime_interaction(
            &room,
            &id,
            "deny",
            None,
            Some(&owner_of(&state, &room)),
        )
        .await
        .unwrap();
    assert!(gate.await.unwrap().is_err());
}

#[test]
fn the_first_use_of_an_approved_request_decides_every_later_try() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (state, _command, _request, _caps, _workspace) = runtime.block_on(async { fixture(false) });
    let spend = |identity: &str, owner: &str, revision: &str| {
        state
            .owned
            .spend_acquisition_send(identity, owner, revision)
            .unwrap()
    };
    assert_eq!(spend("machine-use-a", "o", "v1"), "v1");
    assert_eq!(
        spend("machine-use-a", "o", "v2"),
        "v1",
        "the first use stays"
    );
    assert_eq!(
        spend("machine-use-b", "o", "v3"),
        "v3",
        "another request is its own"
    );
    assert_eq!(
        spend("machine-use-a", "p", "v4"),
        "v4",
        "another owner's use never settles this one"
    );
    // Tries that race agree on one revision.
    let results: Vec<String> = std::thread::scope(|scope| {
        (0..8)
            .map(|n| {
                let state = &state;
                scope.spawn(move || {
                    state
                        .owned
                        .spend_acquisition_send("machine-use-c", "o", &format!("v{n}"))
                        .unwrap()
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect()
    });
    assert!(
        results.iter().all(|result| result == &results[0]),
        "{results:?}"
    );
}

#[tokio::test]
async fn an_approved_try_that_ends_before_the_post_withdraws_its_receipt() {
    let (state, command, request, caps, _workspace) = fixture(false);
    let room = command.session_id.clone().unwrap();
    approve_with(&state, &command, &request, &caps, "approve")
        .await
        .unwrap();
    assert_eq!(receipts(&state, &room).len(), 1);
    // The reauthorization recheck refused the try: the guard drops without posting.
    drop(state.machine_approval_guard(&command, &request).unwrap());
    assert!(receipts(&state, &room).is_empty());
}

#[tokio::test]
async fn an_approved_try_cancelled_while_the_service_answers_settles_its_receipt_as_sent() {
    let (state, command, request, caps, _workspace) = fixture(false);
    let room = command.session_id.clone().unwrap();
    approve_with(&state, &command, &request, &caps, "approve")
        .await
        .unwrap();
    let mut guard = state.machine_approval_guard(&command, &request).unwrap();
    guard.posting();
    // The caller is cancelled with the response delayed: the future and its guard drop.
    drop(guard);
    let held = receipts(&state, &room);
    assert_eq!(held.len(), 1);
    assert_eq!(
        held[0].state,
        crate::session::MachineApprovalReceiptState::Sent
    );
}

#[tokio::test]
async fn a_restart_withdraws_the_receipts_of_tries_it_interrupted() {
    let (state, command, request, caps, _workspace) = fixture(false);
    let room = command.session_id.clone().unwrap();
    approve_with(&state, &command, &request, &caps, "approve")
        .await
        .unwrap();
    assert_eq!(receipts(&state, &room).len(), 1);
    state.withdraw_interrupted_machine_approvals();
    assert!(receipts(&state, &room).is_empty());
}
