//! A finishing child tells its parent once: the `child_finished` event is also the receipt of the
//! delegation, so acknowledging it settles the obligation and no second event is delivered.
use super::child_finished::family;
use crate::durable_state::agent_lifecycle::Operation;

#[tokio::test]
async fn each_finished_child_is_one_inbox_item_and_its_ack_settles_the_delegation() {
    let team = family("child-receipts", 3, Some(false), true).await;
    let store = &team.runtime.owned.durable_state_store;
    // Spawning a child opened one delegation per child on the parent's task.
    let parent_task = store
        .agent_tasks(Some(&team.room), Some(&team.parent))
        .unwrap()
        .remove(0);
    assert_eq!(parent_task.obligations.len(), 3, "{parent_task:?}");
    // The parent prompts each child: the delegation follows that child's task.
    for kid in &team.kids {
        store
            .agent_lifecycle(Operation::BindDelegate {
                parent_task: parent_task.task_id.clone(),
                child_task: kid.task.task_id.clone(),
            })
            .unwrap();
    }
    // The parent ends its turn on its children.
    team.runtime
        .owned
        .complete_local_prompt_without_advance(&team.room, &team.parent, Some(&team.parent_run))
        .unwrap();
    for kid in &team.kids {
        team.runtime
            .finish_agent_task_settlement(
                Some((kid.task.clone(), kid.correction)),
                &kid.child_prompt,
            )
            .await
            .unwrap();
    }

    let read = team
        .runtime
        .read_agent_inbox(&team.room, &team.parent, &serde_json::json!({}))
        .unwrap();
    let kinds: Vec<&str> = read["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["kind"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds, ["child_finished"; 3],
        "one item per child, no separate receipt: {read}"
    );

    for event in read["events"].as_array().unwrap() {
        team.runtime
            .ack_agent_inbox(
                &team.room,
                &team.parent,
                &serde_json::json!({"sequence": event["sequence"], "handled": true}),
                crate::session::unix_epoch_ms(),
            )
            .await
            .unwrap();
    }
    let parent_task = store.agent_task(&parent_task.task_id).unwrap().unwrap();
    let open: Vec<_> = parent_task
        .obligations
        .iter()
        .filter(|obligation| obligation.status != "satisfied")
        .collect();
    assert!(
        open.is_empty(),
        "the acks settle every delegation: {open:?}"
    );
    assert!(
        parent_task.owner_action.is_none()
            && parent_task.state != crate::durable_state::agent_lifecycle::ExecutionState::Blocked,
        "no cancel/resume card is raised: {parent_task:?}"
    );
}

// MP-08 / MP-09 / MP-10 / MP-11 (batchyield3/4): the first inbox call failed, the retry came with `after` = the lead
// sequence and the agent acked through the last row. Every child_finished is still shown and settled, with a
// wait_recheck in the same batch, and no cancel/resume card appears.
#[tokio::test]
async fn a_retry_after_the_lead_still_shows_all_three_children_and_the_ack_through_settles_them() {
    let team = family("child-receipts-retry", 3, Some(false), true).await;
    let store = &team.runtime.owned.durable_state_store;
    let parent_task = store
        .agent_tasks(Some(&team.room), Some(&team.parent))
        .unwrap()
        .remove(0);
    for kid in &team.kids {
        store
            .agent_lifecycle(Operation::BindDelegate {
                parent_task: parent_task.task_id.clone(),
                child_task: kid.task.task_id.clone(),
            })
            .unwrap();
    }
    team.runtime
        .owned
        .complete_local_prompt_without_advance(&team.room, &team.parent, Some(&team.parent_run))
        .unwrap();
    for kid in &team.kids {
        team.runtime
            .finish_agent_task_settlement(
                Some((kid.task.clone(), kid.correction)),
                &kid.child_prompt,
            )
            .await
            .unwrap();
    }
    store
        .agent_lifecycle(Operation::Occur(
            crate::durable_state::agent_lifecycle::occurrence(
                &team.room,
                &team.parent,
                &parent_task.task_id,
                "wait-1-wait_recheck",
                "wait_recheck",
                serde_json::json!({"task_id": parent_task.task_id, "reason": "re-evaluate"}),
            ),
        ))
        .unwrap();
    let lead = store
        .agent_inbox(&team.room, &team.parent, 0)
        .unwrap()
        .into_iter()
        .find(|event| event.kind == "child_finished")
        .unwrap()
        .sequence;
    let read = team
        .runtime
        .read_agent_inbox(
            &team.room,
            &team.parent,
            &serde_json::json!({"after": lead}),
        )
        .unwrap();
    let events = read["events"].as_array().unwrap();
    let finished: Vec<u64> = events
        .iter()
        .filter(|event| event["kind"] == "child_finished")
        .map(|event| event["sequence"].as_u64().unwrap())
        .collect();
    assert_eq!(finished.len(), 3, "all three children are shown: {read}");
    let last = events.last().unwrap()["sequence"].as_u64().unwrap();
    let acked = team
        .runtime
        .ack_agent_inbox(
            &team.room,
            &team.parent,
            &serde_json::json!({"through": last, "handled": true}),
            crate::session::unix_epoch_ms(),
        )
        .await
        .unwrap();
    assert_eq!(
        acked["skipped"],
        serde_json::json!([]),
        "everything was shown: {acked}"
    );
    let parent_task = store.agent_task(&parent_task.task_id).unwrap().unwrap();
    assert!(
        parent_task
            .obligations
            .iter()
            .all(|obligation| obligation.status == "satisfied"),
        "{parent_task:?}"
    );
    assert!(
        parent_task.owner_action.is_none()
            && parent_task.state != crate::durable_state::agent_lifecycle::ExecutionState::Blocked,
        "no cancel/resume card is raised: {parent_task:?}"
    );
}
