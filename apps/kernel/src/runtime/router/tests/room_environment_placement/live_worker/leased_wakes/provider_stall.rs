use super::*;

#[test]
fn provider_stall_on_a_worker_reaches_the_home_parent() {
    run_test(leased_provider_stall_reaches_home_parent);
}

async fn leased_provider_stall_reaches_home_parent() {
    let _tools = with_room_tools();
    let mut fixture = LiveWorker::start().await;
    let worker = start_agent_worker(&fixture, agent_worker_state(&fixture)).await;
    let room = fixture.rooms[0].clone();
    let parent = {
        let mut app = fixture.home.app.lock().await;
        let parent = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                crate::agent::CreateAgentRequest::new(&room, "dev-stub")
                    .with_alias("stall-parent")
                    .with_permission_level_override(crate::provider::AgentPermissionLevel::Yolo),
            )
            .unwrap();
        crate::test_support::admit_room_test_turn(&mut app, &room, parent.id());
        parent
    };
    let runtime = fixture.home.runtime_state();
    let child = runtime
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(&room, "managed-dev-stub")
                .with_parent(&parent)
                .with_alias("leased-child")
                .with_model("native-tui-idle")
                .with_kernel(AGENT_WORKER),
        )
        .await
        .unwrap();
    let attachment = owner_attachment(&fixture, &room).await;
    run_turn(
        &fixture,
        &room,
        &attachment,
        child.id(),
        "Wait on the provider",
    )
    .await;

    let now = crate::session::unix_epoch_ms() + 10 * 60 * 1000 + 5_000;
    for at in [now, now + 1000] {
        runtime.sweep_agent_lifecycle_at_for_test(at).await.unwrap();
    }
    let session = runtime.session_snapshot(&room).await.unwrap();
    let cards = session
        .active_interactions()
        .iter()
        .filter(|card| card.related_agent_id() == Some(child.id()))
        .collect::<Vec<_>>();
    let events = store(&fixture)
        .await
        .agent_inbox(&room, parent.id(), 0)
        .unwrap();
    worker.stop().await;
    fixture.stop().await;

    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].title(), Some("Agent stalled"));
    assert_eq!(cards[0].choices().len(), 3);
    let events = events
        .iter()
        .filter(|event| {
            event.kind == "child_decision" && event.payload["interaction_id"] == cards[0].id()
        })
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 1);
    assert!(events[0].payload["detail"]
        .as_str()
        .unwrap()
        .contains("leased-child"));
}
