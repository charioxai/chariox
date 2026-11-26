//! A machine acquisition is one ordinary pending decision: the person or an agent acting for them answers
//! it, never the agent that asked, and no count of other pending decisions blocks it.
use super::*;
use crate::local::{
    ControlPlaneRequest, JobMachineOffer, JobMachineRate, JobNeeds, KernelConnectionClass,
    LocalDaemonRequest, MachineAllocationCapability,
};
use crate::runtime::command::KernelCommand;
use std::time::Duration;

type Gate = tokio::task::JoinHandle<
    Result<crate::runtime::state::machine_acquisition::Approval, DaemonError>,
>;

/// `requester` asks for a job machine; returns the waiting call and the decision's id.
async fn acquire(t: &Tree, requester: &str) -> (Gate, String) {
    let request = ControlPlaneRequest::AcquireJob {
        capability_revision: "v1".into(),
        request_id: format!("request-{requester}"),
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
    command.agent_id = Some(requester.into());
    command.session_id = Some(t.room.clone());
    command.caller.connection_class = Some(KernelConnectionClass::KernelAgent);
    let capability = MachineAllocationCapability {
        account_machines: None,
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
    let runtime = t.runtime.clone();
    let gate = tokio::spawn(async move {
        runtime
            .approve_machine_acquisition(&command, &request, Some(&capability))
            .await
    });
    let id = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let card = t
                .session()
                .active_interactions()
                .iter()
                .find(|card| card.id().starts_with("machine-acquisition-"))
                .map(|card| card.id().to_owned());
            if let Some(id) = card {
                break id;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the acquisition raised no decision");
    (gate, id)
}

#[tokio::test]
async fn a_parent_answers_an_acquisition_for_the_person_and_the_acquire_proceeds() {
    let t = tree(
        "acquisition-parent",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let (gate, id) = acquire(&t, &t.c).await;
    assert!(
        !t.decision_events(&t.p).is_empty(),
        "the holder was not told"
    );
    assert!(!gate.is_finished(), "the acquire went on before any OK");
    let status = t
        .runtime
        .answer_child_decision(&t.room, &t.p, &id, "approve")
        .await
        .unwrap();
    assert_eq!(status, ChildDecisionStatus::Answered);
    tokio::time::timeout(Duration::from_secs(5), gate)
        .await
        .expect("the acquire kept waiting after an agent answered")
        .unwrap()
        .unwrap();
    let audit = t
        .runtime
        .owned
        .durable_state_store
        .load_subject_events_by_kind(&id, "interaction.answered", 10)
        .unwrap();
    assert_eq!(audit[0].payload["answered_by_agent"], t.p);
}

#[tokio::test]
async fn an_agent_cannot_approve_its_own_acquisition_or_one_its_sub_agents_asked_for() {
    let t = tree(
        "acquisition-self",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let (gate, id) = acquire(&t, &t.p).await;
    for answering in [&t.p, &t.c] {
        let error = t
            .runtime
            .answer_owner_room_card(answering, &t.room, &id, "approve")
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("cannot approve a machine acquisition"),
            "{error}"
        );
    }
    assert!(
        !gate.is_finished(),
        "a self-approval let the acquisition go on"
    );
    let status = t
        .runtime
        .answer_owner_room_card(&t.g, &t.room, &id, "approve")
        .unwrap();
    assert_eq!(status, ChildDecisionStatus::Answered);
    tokio::time::timeout(Duration::from_secs(5), gate)
        .await
        .expect("the acquire kept waiting after another agent answered")
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn an_agent_in_another_room_below_the_requester_cannot_approve_its_acquisition() {
    let t = tree(
        "acquisition-cross-room",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    // Two more Rooms of the same owner: the first Room's agent was spawned by the requester `p` and
    // keeps it as its parent; the second Room's agent was spawned by that one (a chain across Rooms).
    let (below, deeper, _worktrees) = {
        let mut app = t.app.lock().await;
        let mut ids = Vec::new();
        let mut worktrees = Vec::new();
        let mut parent = t.p.clone();
        for label in ["acquisition-cross-room-b", "acquisition-cross-room-c"] {
            let worktree = crate::test_support::TestWorktree::new(label);
            let (room, agent) = crate::app::KernelSessionService::new(&mut app)
                .create_session(worktree.session_request())
                .unwrap();
            crate::test_support::admit_room_test_turn(&mut app, room.id(), agent.id());
            app.agents()
                .set_parent_agent(agent.id(), Some(crate::agent::AgentRef::local(&parent)))
                .unwrap();
            parent = agent.id().to_owned();
            ids.push(agent.id().to_owned());
            worktrees.push(worktree);
        }
        (ids[0].clone(), ids[1].clone(), worktrees)
    };
    let (gate, id) = acquire(&t, &t.p).await;
    for answering in [&below, &deeper] {
        let error = t
            .runtime
            .answer_owner_room_card(answering, &t.room, &id, "approve")
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("cannot approve a machine acquisition"),
            "an agent below the requester in another Room was let answer: {error}"
        );
    }
    assert!(
        !gate.is_finished(),
        "a cross-Room self-approval let the acquisition go on"
    );
    // An agent outside the requester's subtree still can.
    let status = t
        .runtime
        .answer_owner_room_card(&t.g, &t.room, &id, "approve")
        .unwrap();
    assert_eq!(status, ChildDecisionStatus::Answered);
}

#[tokio::test]
async fn many_pending_decisions_do_not_block_an_acquisition() {
    let t = tree(
        "acquisition-many",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let owner = t.session().owner_user_id().to_owned();
    let mut pending = Vec::new();
    for index in 0..40 {
        let id = format!("pending-{index}");
        pending.push(
            t.runtime
                .create_kernel_operation_interaction(
                    crate::runtime::state::CardKind::OwnerControl,
                    &t.room,
                    &owner,
                    RuntimeInteraction::for_kernel_operation(
                        &id,
                        &id,
                        "Pending",
                        "Waiting for an answer",
                        vec![RuntimeInteractionChoice::new(
                            "cancel", "Cancel", "cancel", None,
                        )],
                    )
                    .with_timeout_sec(900),
                )
                .await
                .unwrap(),
        );
    }
    let (gate, id) = acquire(&t, &t.c).await;
    t.runtime
        .answer_child_decision(&t.room, &t.p, &id, "approve")
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), gate)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    drop(pending);
}
