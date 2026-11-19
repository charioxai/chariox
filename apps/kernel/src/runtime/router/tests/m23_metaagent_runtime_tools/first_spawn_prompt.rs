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
