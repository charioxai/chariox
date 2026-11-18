// An agent changes its own settings through the room command, inside the agent tree's policy.
// Tree from `world`: a -> {b -> {d}, s}.
use super::subtree::{error_text, room_command, world, World};
use super::*;

async fn allow_models(world: &World, agent: &str, models: &[&str]) {
    world
        .app
        .lock()
        .await
        .agents()
        .set_agent_self_settings(
            &world.ids[agent],
            crate::agent::AgentSelfSettings {
                models: models.iter().map(|model| model.to_string()).collect(),
                ..Default::default()
            },
        )
        .unwrap();
}

async fn model_of(world: &World, agent: &str) -> Option<String> {
    world
        .app
        .lock()
        .await
        .agents()
        .get_agent(&world.ids[agent])
        .unwrap()
        .model()
        .map(str::to_string)
}

#[test]
fn an_agent_changes_its_own_model_inside_the_set_with_the_room_command() {
    run_large_stack_async_test(
        "self-settings-in-set",
        an_agent_changes_its_own_model_inside_the_set_with_the_room_command_body,
    );
}

async fn an_agent_changes_its_own_model_inside_the_set_with_the_room_command_body() {
    let world = world("self-settings-in-set").await;
    allow_models(&world, "b", &["m-x"]).await;

    let result = room_command(
        &world,
        "b",
        "agent settings set --model m-x --reason 'needs more depth'",
    )
    .await;

    assert!(result.ok, "{result:?}");
    assert_eq!(
        result.payload["response"]["type"], "AgentSettingsChanged",
        "{result:?}"
    );
    assert_eq!(result.payload["response"]["settings"]["model"], "m-x");
    // The agent makes the call during its own turn, so the change reaches its process at the
    // step that ends this call.
    assert_eq!(result.payload["response"]["status"], "applied, continuing");
    assert_eq!(model_of(&world, "b").await.as_deref(), Some("m-x"));
}

#[test]
fn an_agent_reaches_itself_and_the_agents_below_it_and_no_other() {
    run_large_stack_async_test(
        "self-settings-reach",
        an_agent_reaches_itself_and_the_agents_below_it_and_no_other_body,
    );
}

async fn an_agent_reaches_itself_and_the_agents_below_it_and_no_other_body() {
    let world = world("self-settings-reach").await;
    allow_models(&world, "b", &["m-y"]).await;

    // The agent above chooses a child's settings without the child's set.
    let above = room_command(&world, "a", "agent settings set b --model m-below").await;
    assert!(above.ok, "{above:?}");
    assert_eq!(model_of(&world, "b").await.as_deref(), Some("m-below"));

    // A child cannot reach its sibling or the agent above it.
    for target in ["s", "a"] {
        let refused = room_command(
            &world,
            "b",
            &format!("agent settings set {target} --model m-y"),
        )
        .await;
        assert!(!refused.ok, "b must not change {target}: {refused:?}");
        assert!(!error_text(&refused).is_empty(), "{refused:?}");
    }
}

#[test]
fn an_agent_cannot_take_its_own_usage_limit_decision() {
    run_large_stack_async_test(
        "self-settings-resume",
        an_agent_cannot_take_its_own_usage_limit_decision_body,
    );
}

async fn an_agent_cannot_take_its_own_usage_limit_decision_body() {
    let world = world("self-settings-resume").await;

    let own = room_command(&world, "b", "agent resume b").await;
    assert!(!own.ok, "{own:?}");

    let not_stopped = room_command(&world, "a", "agent resume b").await;
    assert!(!not_stopped.ok, "{not_stopped:?}");
    assert!(
        error_text(&not_stopped).contains("not stopped by a usage limit"),
        "{not_stopped:?}"
    );
}
