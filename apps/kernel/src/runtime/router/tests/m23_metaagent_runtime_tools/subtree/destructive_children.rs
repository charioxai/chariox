use super::*;

// Row 405 (PA11): an agent above, not only the direct parent, renames and deletes a live child.
#[test]
fn an_ancestor_renames_and_deletes_a_live_descendant() {
    run_large_stack_async_test("subtree-direct-destructive-child", direct_destructive_child);
}

async fn direct_destructive_child() {
    let world = world("subtree-direct-destructive-child").await;
    let renamed = room_command(&world, "a", "agent alias d renamed-by-grandparent").await;
    assert!(renamed.ok, "{renamed:?}");
    let deleted = room_command(&world, "a", &format!("agent delete {}", world.ids["d"])).await;
    assert!(deleted.ok, "{deleted:?}");
    let app = world.app.lock().await;
    assert!(app.agents().get_agent(&world.ids["d"]).is_err());
    for retained in ["a", "b", "s"] {
        assert!(app.agents().get_agent(&world.ids[retained]).is_ok());
    }
}
