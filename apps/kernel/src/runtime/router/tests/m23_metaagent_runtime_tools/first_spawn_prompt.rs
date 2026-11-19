use super::subtree::{room_command, world};
use super::*;

#[test]
fn cross_room_spawn_admits_one_first_prompt() {
    run_large_stack_async_test("first-spawn-prompt", || async {
        let world = world("first-spawn-prompt").await;
        let task = "create spawntask-marker.txt with text SPAWNTASK then report";
        let command = format!(
            "agent spawn started --provider dev-stub --room {} --task '{task}'",
            world.other_room
        );
        let spawned = room_command(&world, "a", &command).await;
        assert!(spawned.ok, "{spawned:?}");
        let id = spawned.payload["response"]["agent"]["id"].as_str().unwrap();
        let tasks = world
            .router
            .runtime_state
            .durable_state_store()
            .agent_tasks(Some(&world.other_room), Some(id))
            .unwrap();
        assert_eq!(tasks.len(), 1, "spawn must admit the first prompt itself");
        let author = world
            .app
            .lock()
            .await
            .agents()
            .get_agent(&world.ids["a"])
            .unwrap();
        let author_name = author.alias().unwrap_or_else(|| author.agent_ref());
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let outline =
                    crate::runtime::history_requests::execute_session_history_outline_request(
                        world.router.operational_history_store.clone(),
                        crate::local::GetSessionHistoryOutlineRequest {
                            session_id: world.other_room.clone(),
                            agent_ids: Some(vec![id.to_string()]),
                            latest_prompt_count: Some(1),
                            cursor: None,
                            include_prefetched_content: false,
                        },
                    )
                    .await
                    .unwrap();
                let LocalDaemonResponse::SessionHistoryOutline { agents } = outline else {
                    panic!("unexpected child history response");
                };
                if let Some(turn) = agents.first().and_then(|agent| agent.turns.first()) {
                    assert_eq!(turn.prompt_id.as_deref(), Some(tasks[0].prompt_id.as_str()));
                    assert_eq!(turn.user_prompt.entry.text.trim_end(), task);
                    assert_eq!(
                        turn.user_prompt.entry.prompt_author.as_deref(),
                        Some(author_name)
                    );
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("first prompt must reach the child transcript");
    });
}

#[test]
fn cross_room_first_spawn_prompt_binds_and_settles_the_delegate() {
    run_large_stack_async_test("first-spawn-delegation", || async {
        use crate::durable_state::agent_lifecycle::ExecutionState;

        let world = world("first-spawn-delegation").await;
        let runtime = &world.router.runtime_state;
        let store = runtime.durable_state_store();
        let parent = &world.ids["peer"];
        let command = format!(
            "agent spawn delegated --provider dev-stub --room {} --task 'review the changes'",
            world.other_room
        );
        let spawned = room_command(&world, "peer", &command).await;
        assert!(spawned.ok, "{spawned:?}");
        let child = spawned.payload["response"]["agent"]["id"].as_str().unwrap();
        let parent_task = store
            .agent_tasks(Some(&world.room), Some(parent))
            .unwrap()
            .remove(0);
        let child_task = store
            .agent_tasks(Some(&world.other_room), Some(child))
            .unwrap()
            .remove(0);
        let obligation = parent_task
            .obligations
            .iter()
            .find(|obligation| obligation.resource_id.as_deref() == Some(child))
            .expect("spawn registers the parent's delegate obligation");
        assert_eq!(
            obligation.completion_task_id.as_deref(),
            Some(child_task.task_id.as_str()),
            "the obligation follows the child's first prompt"
        );

        runtime
            .end_agent_turn_for_tests(&world.other_room, child, false)
            .await
            .unwrap();
        let event = store
            .agent_inbox(&world.room, parent, 0)
            .unwrap()
            .into_iter()
            .find(|event| event.kind == "child_finished")
            .expect("the parent's inbox records the child's result");
        assert_eq!(event.payload["child_task_id"], child_task.task_id);
        if event.state != "handled" {
            let ack = world
                .router
                .dispatch_authenticated_runtime_tool_call(
                    &world.tokens["peer"],
                    "chariox.events.ack",
                    serde_json::json!({
                        "task_id": parent_task.task_id,
                        "origin_prompt_id": parent_task.prompt_id,
                        "sequence": event.sequence,
                        "handled": true,
                    }),
                )
                .await
                .unwrap();
            assert!(ack.ok, "{ack:?}");
        }
        runtime
            .end_agent_turn_for_tests(&world.room, parent, false)
            .await
            .unwrap();
        let settled = store.agent_task(&parent_task.task_id).unwrap().unwrap();
        assert_eq!(settled.state, ExecutionState::Done, "{settled:?}");
        assert!(settled.owner_action.is_none(), "{settled:?}");
        assert_eq!(
            settled
                .obligations
                .iter()
                .find(|entry| entry.id == obligation.id)
                .unwrap()
                .status,
            "satisfied"
        );
    });
}
