// A Room command reply answers the command: `prompt` returns the target's outcome and
// `agent children` one compact row per child, however many agents the Room holds.
use super::waiting_children::yield_on_a_labelled_timer;
use super::*;

const CHILDREN: usize = 60;

/// `a` gets `CHILDREN` more children, each waiting on a long shell command.
async fn crowded_world(label: &str) -> World {
    let mut world = world(label).await;
    for index in 0..CHILDREN {
        let alias: &'static str = Box::leak(format!("kid{index}").into_boxed_str());
        let spawned = room_command(
            &world,
            "a",
            &format!("agent spawn {alias} --provider dev-stub"),
        )
        .await;
        assert!(spawned.ok, "{spawned:?}");
        world.ids.insert(
            alias,
            spawned.payload["response"]["agent"]["id"]
                .as_str()
                .unwrap()
                .to_owned(),
        );
    }
    world
}

#[test]
fn prompting_one_agent_in_a_crowded_room_replies_with_its_outcome_only() {
    run_large_stack_async_test("subtree-prompt-reply", prompt_reply);
}

async fn prompt_reply() {
    let world = crowded_world("subtree-prompt-reply").await;
    let sent = room_command(&world, "a", "prompt b read the README").await;
    assert!(sent.ok, "{sent:?}");
    let size = serde_json::to_vec(&sent.payload).unwrap().len();
    assert!(size < 1024, "{size} bytes: {}", sent.payload);
    assert_eq!(sent.payload["target_agent_id"], world.ids["b"]);
    assert!(sent.payload["prompt_id"].as_str().is_some(), "{sent:?}");
    assert!(sent.payload["status"].as_str().is_some(), "{sent:?}");
    assert!(sent.payload.get("agent_activity").is_none(), "{sent:?}");
}

#[test]
fn agent_children_lists_every_child_compactly_and_agent_get_keeps_the_detail() {
    run_large_stack_async_test("subtree-children-reply", children_reply);
}

async fn children_reply() {
    let world = crowded_world("subtree-children-reply").await;
    let wait = "while true; do curl -s https://example.test/status | jq .ready; sleep 30; done # a long background command body that must not fill the reply";
    for index in 0..CHILDREN {
        let alias = format!("kid{index}");
        yield_on_a_labelled_timer(
            &world,
            &alias,
            crate::session::unix_epoch_ms() - 60_000,
            wait,
        )
        .await;
    }
    let children = room_command(&world, "a", "agent children").await;
    assert!(children.ok, "{children:?}");
    let size = serde_json::to_vec(&children.payload).unwrap().len();
    assert!(size < 8 * 1024, "{size} bytes");
    let listed = children.payload["response"]["children"].as_array().unwrap();
    assert_eq!(listed.len(), CHILDREN + 3, "every child is listed");
    let row = listed
        .iter()
        .find(|child| child["alias"] == "kid7")
        .unwrap();
    assert_eq!(row["state"], "waiting", "{row}");
    assert!(row["wait"].as_str().unwrap().chars().count() < 40, "{row}");
    assert!(row.get("liveness").is_none());

    let inspected = world
        .router
        .dispatch_authenticated_runtime_tool_call(
            &world.tokens["a"],
            crate::transport::runtime_tools::GET_SESSION_AGENT_TOOL,
            serde_json::json!({ "agent": "kid7" }),
        )
        .await
        .unwrap();
    assert!(inspected.ok, "{inspected:?}");
    let event = &inspected.payload["agent"]["liveness"]["waiting_events"][0];
    assert!(event["label"]
        .as_str()
        .unwrap()
        .contains("background command body"));
}
