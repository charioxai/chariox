//! A person answers the first watched-command card "Allow for this agent": the agent's later watched
//! commands run without a card until a parent revokes the grant.
use super::tests::{fixture_with_run_profile, running, Fixture};
use crate::durable_state::agent_lifecycle::{Operation, Outcome};
use crate::provider::{AgentExecutionMode, AgentPermissionLevel};
use crate::session::RuntimeInteraction;
use std::time::Duration;

async fn watched_command_card(f: &Fixture, session_id: &str) -> RuntimeInteraction {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(card) = f
                .state
                .owned
                .session_snapshot(session_id)
                .unwrap()
                .active_interactions()
                .iter()
                .find(|interaction| interaction.id().starts_with("watched-process-"))
                .cloned()
            {
                break card;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("a watched command without a grant must ask")
}

/// The runtime token of a new agent of the room that is the fixture agent's parent.
async fn parent_of_fixture_agent(f: &Fixture, session_id: &str, agent_id: &str) -> String {
    let mut app = f.app.lock().await;
    let parent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session_id, "dev-stub").with_alias("grantor"),
        )
        .unwrap();
    let run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session_id,
                "dev-stub",
                "dev-stub",
                "default",
                "room-model",
            )
            .with_agent_id(parent.id()),
        )
        .unwrap();
    app.update_provider_run_projection(run);
    crate::test_support::admit_room_test_turn(&mut app, session_id, parent.id());
    app.agents_mut()
        .set_parent_agent(agent_id, Some(crate::agent::AgentRef::local(parent.id())))
        .unwrap();
    app.providers()
        .get_run_for_agent(session_id, parent.id())
        .unwrap()
        .runtime_mcp_auth_token()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn allow_for_this_agent_ends_the_cards_until_a_parent_revokes_it() {
    let f = fixture_with_run_profile(None, true, "claude", "claude");
    let turn = running(&f);
    f.state.owned.sudo_turns.lock().unwrap().clear();
    f.state
        .owned
        .provider_store
        .update_run_execution_config(
            f.run.id(),
            AgentExecutionMode::Build,
            AgentPermissionLevel::Yolo,
        )
        .unwrap();
    let Outcome::Task(task) = f
        .state
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::Begin {
            owner: turn.owner_user_id.clone(),
            room: turn.session_id.clone(),
            agent: turn.agent_id.clone(),
            prompt: turn.prompt_id.clone().unwrap(),
            run: Some(f.run.id().into()),
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap()
    else {
        panic!("task missing")
    };
    let parent_token = parent_of_fixture_agent(&f, &turn.session_id, &turn.agent_id).await;
    let watch = |label: &'static str| {
        f.router.dispatch_authenticated_runtime_tool_call(
            "sudo-fixture-bearer",
            "chariox.events.process",
            serde_json::json!({"task_id":task.task_id, "origin_prompt_id":turn.prompt_id,
                "label":label, "argv":["/bin/true"]}),
        )
    };
    let room_command = |command: String| {
        let parent_token = parent_token.clone();
        let router = f.router.clone();
        async move {
            router
                .dispatch_authenticated_runtime_tool_call(
                    &parent_token,
                    "chariox.room.run_command",
                    serde_json::json!({"command": command}),
                )
                .await
                .unwrap()
        }
    };

    // The first watched command asks, and the card offers the standing grant.
    let (first, ()) = tokio::join!(watch("first"), async {
        let card = watched_command_card(&f, &turn.session_id).await;
        let choices: Vec<_> = card.choices().iter().map(|c| c.id().to_owned()).collect();
        assert_eq!(choices, ["allow_once", "allow_agent", "deny"]);
        f.state
            .resolve_runtime_interaction(&turn.session_id, card.id(), "allow_agent", None)
            .await
            .unwrap();
    });
    first.expect("the granted watched command runs");

    // The second one runs without a card.
    tokio::time::timeout(Duration::from_secs(5), watch("second"))
        .await
        .expect("a granted agent's watched command must not wait for a card")
        .expect("the second watched command runs");
    assert!(f
        .state
        .owned
        .session_snapshot(&turn.session_id)
        .unwrap()
        .active_interactions()
        .iter()
        .all(|interaction| !interaction.id().starts_with("watched-process-")));

    // The parent sees the grant, and revokes it.
    let children = room_command("agent children".into()).await;
    assert_eq!(
        children.payload["response"]["children"][0]["grants"],
        serde_json::json!(["watched-process"]),
        "{children:?}"
    );
    let revoked = room_command("agent revoke grantor watched-process".into()).await;
    assert!(
        !revoked.ok,
        "the parent cannot revoke from itself, it is no agent below it: {revoked:?}"
    );
    let revoked = room_command(format!("agent revoke {} watched-process", turn.agent_id)).await;
    assert!(revoked.ok, "{revoked:?}");
    assert_eq!(revoked.payload["response"]["was_held"], true);
    let children = room_command("agent children".into()).await;
    assert_eq!(
        children.payload["response"]["children"][0]["grants"],
        serde_json::json!([])
    );

    // The next watched command asks again.
    let (third, ()) = tokio::join!(watch("third"), async {
        let card = watched_command_card(&f, &turn.session_id).await;
        f.state
            .resolve_runtime_interaction(&turn.session_id, card.id(), "deny", None)
            .await
            .unwrap();
    });
    assert!(third.is_err(), "a denied watched command must not run");
    f.state.owned.agent_wakes.processes.shutdown();
}
