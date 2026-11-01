//! MP-08 / MP-09 / MP-10 / MP-11: ended tasks release the owner decision board.
use super::*;
use crate::durable_state::agent_lifecycle::{Operation, Outcome};

async fn blocked_tasks(
    f: &Fixture,
) -> Vec<crate::durable_state::agent_lifecycle::AgentTaskExecution> {
    let store = &f.state.owned.durable_state_store;
    let mut tasks = Vec::new();
    for index in 0..8 {
        let id = f.id(&format!("blocked-{index}"));
        store
            .agent_lifecycle(Operation::Begin {
                owner: DEFAULT_LOCAL_USER_ID.into(),
                room: f.session.clone(),
                agent: format!("agent-{index}"),
                prompt: id.clone(),
                run: None,
                now: 1,
            })
            .unwrap();
        let Outcome::Task(task) = store
            .agent_lifecycle(Operation::Block {
                task: id.clone(),
                prompt: id,
                reason: "Owner decision required".into(),
            })
            .unwrap()
        else {
            panic!()
        };
        f.state
            .ensure_task_owner_interaction(task.clone())
            .await
            .unwrap();
        tasks.push(task);
    }
    tasks
}

#[tokio::test]
async fn a12_cancelled_task_decisions_release_capacity_before_the_next_sweep() {
    let f = Fixture::with_room_tools();
    let tasks = blocked_tasks(&f).await;
    assert_eq!(f.active_ids().len(), 8);
    for task in tasks {
        f.state
            .owned
            .durable_state_store
            .agent_lifecycle(Operation::CancelTask {
                task: task.task_id,
                owner: DEFAULT_LOCAL_USER_ID.into(),
                revision: task.revision,
            })
            .unwrap();
    }
    let receiver = f.register("new-owner-operation");
    assert!(
        receiver.is_ok(),
        "ended tasks must release capacity at admission: {:?}",
        receiver.err()
    );
    assert_eq!(f.active_ids(), vec![f.id("new-owner-operation")]);
}

#[tokio::test]
async fn a12_task_sweep_settles_ended_decisions() {
    let f = Fixture::with_room_tools();
    let mut tasks = blocked_tasks(&f).await;
    let task = tasks.remove(0);
    let store = &f.state.owned.durable_state_store;
    store
        .agent_lifecycle(Operation::OwnerResponse {
            task: task.task_id.clone(),
            revision: task.blocked_revision,
            resume: false,
            now: 2,
        })
        .unwrap();
    f.state.owned.sweep_kernel_operation_interactions(false);
    assert_eq!(
        f.active_ids().len(),
        7,
        "the ordinary interaction sweep must settle ended task cards"
    );
    assert!(!f.active_ids().iter().any(|id| id.contains(&task.task_id)));
}

#[tokio::test]
async fn a12_reblocked_task_replaces_its_card_without_cancelling_the_new_revision() {
    let f = Fixture::with_room_tools();
    let task = blocked_tasks(&f).await.remove(0);
    let old_card = format!("task-blocked-{}-{}", task.task_id, task.blocked_revision);
    let store = &f.state.owned.durable_state_store;
    let Outcome::Task(resumed) = store
        .agent_lifecycle(Operation::OwnerResponse {
            task: task.task_id.clone(),
            revision: task.blocked_revision,
            resume: true,
            now: 2,
        })
        .unwrap()
    else {
        panic!()
    };
    let prompt = resumed.pending_prompt_id.unwrap();
    store
        .agent_lifecycle(Operation::Begin {
            owner: DEFAULT_LOCAL_USER_ID.into(),
            room: f.session.clone(),
            agent: task.agent_id.clone(),
            prompt: prompt.clone(),
            run: None,
            now: 3,
        })
        .unwrap();
    let Outcome::Task(reblocked) = store
        .agent_lifecycle(Operation::Block {
            task: task.task_id.clone(),
            prompt,
            reason: "A different owner decision is required".into(),
        })
        .unwrap()
    else {
        panic!()
    };
    f.state
        .ensure_task_owner_interaction(reblocked.clone())
        .await
        .unwrap();
    tokio::task::yield_now().await;
    assert_eq!(f.active_ids().len(), 8);
    assert!(!f.active_ids().contains(&old_card));
    assert!(f.active_ids().contains(&format!(
        "task-blocked-{}-{}",
        task.task_id, reblocked.blocked_revision
    )));
    let latest = store
        .agent_tasks(Some(&f.session), Some(&task.agent_id))
        .unwrap();
    assert_eq!(latest[0].state, reblocked.state);
    assert_eq!(latest[0].blocked_revision, reblocked.blocked_revision);
}

#[tokio::test]
async fn a12_task_end_withdraws_a_restart_restored_card_without_a_responder() {
    let f = Fixture::with_room_tools();
    let task = blocked_tasks(&f).await.remove(0);
    let card = format!("task-blocked-{}-{}", task.task_id, task.blocked_revision);
    f.state.owned.pending_interactions.write().remove(&card);
    let Outcome::Task(cancelled) = f
        .state
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::CancelTask {
            task: task.task_id,
            owner: DEFAULT_LOCAL_USER_ID.into(),
            revision: task.revision,
        })
        .unwrap()
    else {
        panic!()
    };
    f.state
        .retract_stale_task_owner_interactions(&f.session, &[cancelled])
        .await;
    assert_eq!(
        f.active_ids().len(),
        7,
        "the normal task-end projection must also withdraw restored metadata"
    );
    assert!(!f.active_ids().contains(&card));
}

#[tokio::test]
async fn a12_refused_replacement_still_settles_the_abandoned_card() {
    let f = Fixture::new();
    let orphan = f.register("orphan").unwrap();
    f.state
        .owned
        .pending_interactions
        .write()
        .remove(&f.id("orphan"));
    drop(orphan);
    let mut receivers = Vec::new();
    for index in 0..8 {
        receivers.push(f.register(&format!("live-{index}")).unwrap());
    }
    assert_eq!(f.active_ids().len(), 9);
    assert!(
        f.register("orphan").is_err(),
        "the live owner quota remains enforced"
    );
    assert_eq!(
        f.active_ids().len(),
        8,
        "a refused admission must commit both sides of superseded cleanup"
    );
    assert!(!f.active_ids().contains(&f.id("orphan")));
    drop(receivers);
}

#[tokio::test]
async fn a12_owner_elevation_reserves_the_last_slot_and_keeps_the_global_bound() {
    let f = Fixture::new();
    let owners = [DEFAULT_LOCAL_USER_ID, "owner-a", "owner-b", "owner-c"];
    {
        let mut sessions = f.state.owned.session_store.write();
        let mut session = sessions.get_session(&f.session).unwrap();
        for owner in &owners[1..] {
            session.add_member(*owner, None, crate::session::CollaborationLevel::Full);
        }
        sessions.restore_session(session);
    }
    let mut receivers = Vec::new();
    for index in 0..31 {
        let id = f.id(&format!("global-{index}"));
        receivers.push(
            f.state
                .create_kernel_operation_interaction(
                    &f.session,
                    owners[index / 8],
                    RuntimeInteraction::for_kernel_operation(
                        &id,
                        &id,
                        "Agent decision",
                        "Choose an action",
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
    let id = f.id("agent-over-reservation");
    assert!(
        f.state
            .create_kernel_operation_interaction(
                &f.session,
                owners[3],
                RuntimeInteraction::for_kernel_operation(
                    &id,
                    &id,
                    "Agent decision",
                    "Choose an action",
                    vec![RuntimeInteractionChoice::new(
                        "cancel", "Cancel", "cancel", None
                    ),]
                )
                .with_timeout_sec(900),
            )
            .await
            .is_err(),
        "agent requests cannot spend the reserved final slot"
    );
    let elevation = |suffix: &str| {
        let id = format!("sudo:{}:{suffix}", f.session);
        RuntimeInteraction::for_kernel_operation(
            &id,
            &id,
            "Run one sudo task",
            "Owner elevation",
            vec![
                RuntimeInteractionChoice::new("refuse", "Refuse", "refuse", None),
                RuntimeInteractionChoice::new("approve", "Approve", "approve", None)
                    .requiring_passkey(),
            ],
        )
        .with_timeout_sec(900)
    };
    receivers.push(
        f.state
            .create_kernel_operation_interaction(&f.session, owners[3], elevation("last-slot"))
            .await
            .unwrap(),
    );
    assert_eq!(f.active_ids().len(), 32);
    assert!(
        f.state
            .create_kernel_operation_interaction(&f.session, owners[0], elevation("over-limit"))
            .await
            .is_err(),
        "owner elevation also obeys the hard32 bound"
    );
    assert_eq!(f.active_ids().len(), 32);
    drop(receivers);
}
