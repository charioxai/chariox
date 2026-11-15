use super::*;

#[test]
fn only_a_direct_parent_can_rename_or_delete_a_live_child() {
    run_large_stack_async_test("subtree-direct-destructive-child", direct_destructive_child);
}

async fn direct_destructive_child() {
    let world = world("subtree-direct-destructive-child").await;
    for command in ["agent alias d renamed-by-grandparent", "agent delete d"] {
        let refused = room_command(&world, "a", command).await;
        assert!(!refused.ok, "grandparent admitted {command}: {refused:?}");
        assert!(error_text(&refused).contains("direct child"), "{refused:?}");
        let app = world.app.lock().await;
        let child = app.agents().get_agent(&world.ids["d"]).unwrap();
        assert_eq!(child.parent_agent_id(), Some(world.ids["b"].as_str()));
        assert_eq!(child.alias(), Some("d"));
    }
    let renamed = room_command(&world, "b", "agent alias d renamed-by-parent").await;
    assert!(renamed.ok, "{renamed:?}");
    let deleted = room_command(&world, "b", &format!("agent delete {}", world.ids["d"])).await;
    assert!(deleted.ok, "{deleted:?}");
    let app = world.app.lock().await;
    assert!(app.agents().get_agent(&world.ids["d"]).is_err());
    for retained in ["a", "b", "s"] {
        assert!(app.agents().get_agent(&world.ids[retained]).is_ok());
    }
}
