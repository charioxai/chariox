// An agent changes its own settings through the room command, inside the agent tree's policy.
// Tree from `world`: a -> {b -> {d}, s}.
use super::subtree::{error_text, room_command, world, World};
use super::*;

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
fn an_agent_changes_its_own_model_with_the_room_command() {
    run_large_stack_async_test(
        "self-settings-own-model",
        an_agent_changes_its_own_model_with_the_room_command_body,
    );
}

async fn an_agent_changes_its_own_model_with_the_room_command_body() {
    let world = world("self-settings-own-model").await;

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
    // The agent above chooses a child's settings.
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

#[test]
fn agent_profile_turn_end_reports_pending_to_the_cli_and_requesting_agent() {
    run_large_stack_async_test("settings-at-turn-end", || async {
        let world = world("settings-at-turn-end").await;
        let result = room_command(
            &world,
            "b",
            "agent settings set --model m-next --at-turn-end",
        )
        .await;
        assert!(result.ok, "{result:?}");
        assert_eq!(
            result.payload["response"]["status"],
            "pending until this turn ends"
        );
        assert_ne!(model_of(&world, "b").await.as_deref(), Some("m-next"));

        let read = room_command(&world, "b", "agent settings").await;
        assert!(read.ok, "{read:?}");
        assert_eq!(read.payload["response"]["pending"]["model"], "m-next");

        let read = LocalDaemonRequest::GetAgentSettings(crate::local::GetAgentSettingsRequest {
            session_id: Some(world.room.clone()),
            agent: "b".into(),
        });
        let command = KernelCommand::from_local_request("read-agent-settings", None, None, &read);
        let LocalDaemonResponse::AgentSettings { agent } =
            world.router.dispatch(command, read).await.unwrap()
        else {
            panic!("the CLI can read the sub-agent's settings");
        };
        assert_eq!(agent.id, world.ids["b"]);
        assert_eq!(
            agent.pending_profile.unwrap().model.as_deref(),
            Some("m-next")
        );

        let refused = room_command(&world, "b", "agent settings s").await;
        assert!(!refused.ok, "{refused:?}");
    });
}
