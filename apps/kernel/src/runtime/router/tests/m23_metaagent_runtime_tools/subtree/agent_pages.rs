// R3: a Room of many agents lists them as compact rows in pages, so a successor coordinator
// can read its predecessor's lanes and find its parent's alias in one small reply.
use super::waiting_children::yield_on_a_timer;
use super::*;
use crate::transport::runtime_tools::{
    GET_SESSION_AGENT_TOOL, LIST_SESSION_AGENTS_TOOL, META_SESSION_OVERVIEW_TOOL,
};

const AGENTS: usize = 90;
/// `a`, `b`, `d`, `s` and `peer`, besides the lanes.
const FIXED_AGENTS: usize = 5;

/// A room agent's team holds at most 64 sub-agents, so the lanes come from two room agents.
async fn crowded_world(label: &str) -> World {
    let mut world = world(label).await;
    for index in 0..AGENTS {
        let alias: &'static str = Box::leak(format!("lane{index}").into_boxed_str());
        let parent = if index < AGENTS / 2 { "a" } else { "peer" };
        let spawned = room_command(
            &world,
            parent,
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

async fn call(
    world: &World,
    who: &str,
    tool: &str,
    arguments: serde_json::Value,
) -> serde_json::Value {
    let result = world
        .router
        .dispatch_authenticated_runtime_tool_call(&world.tokens[who], tool, arguments)
        .await
        .unwrap();
    assert!(result.ok, "{result:?}");
    result.payload
}

/// The room agent `a` has no chosen alias: rows name it by the alias it was given.
async fn room_agent_alias(world: &World) -> String {
    let page = call(
        world,
        "b",
        LIST_SESSION_AGENTS_TOOL,
        serde_json::json!({ "alias": "agent" }),
    )
    .await;
    page["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == world.ids["a"])
        .map(|row| row["alias"].as_str().unwrap().to_owned())
        .expect("the room agent is listed under its alias")
}

fn size(payload: &serde_json::Value) -> usize {
    serde_json::to_vec(payload).unwrap().len()
}

#[test]
fn list_session_agents_answers_a_crowded_room_in_small_pages_of_compact_rows() {
    run_large_stack_async_test("agent-pages-list", list_pages);
}

async fn list_pages() {
    let world = crowded_world("agent-pages-list").await;
    let a_alias = room_agent_alias(&world).await;
    let mut listed = Vec::new();
    let mut cursor: Option<String> = None;
    let mut first_page_bytes = 0;
    loop {
        let arguments = cursor.as_ref().map_or_else(
            || serde_json::json!({}),
            |cursor| serde_json::json!({ "cursor": cursor }),
        );
        let page = call(&world, "b", LIST_SESSION_AGENTS_TOOL, arguments).await;
        if listed.is_empty() {
            first_page_bytes = size(&page);
            // R3a: the agent finds its parent's alias without reading anything else.
            assert_eq!(page["you"]["alias"], "b", "{page}");
            assert_eq!(page["you"]["parent"], a_alias, "{page}");
        }
        assert!(page.get("agent_tasks").is_none(), "task history stays out");
        for row in page["agents"].as_array().unwrap() {
            for key in row.as_object().unwrap().keys() {
                assert!(
                    ["id", "alias", "state", "turn", "provider", "model", "parent"]
                        .contains(&key.as_str()),
                    "{row}"
                );
            }
            listed.push(row.clone());
        }
        assert_eq!(page["total"], AGENTS + FIXED_AGENTS);
        match page["next_cursor"].as_str() {
            Some(next) => cursor = Some(next.to_owned()),
            None => break,
        }
    }
    assert!(first_page_bytes < 16 * 1024, "{first_page_bytes} bytes");
    assert_eq!(
        listed.len(),
        AGENTS + FIXED_AGENTS,
        "every agent is on some page"
    );
    let lane = listed.iter().find(|row| row["alias"] == "lane7").unwrap();
    assert_eq!(
        lane["parent"], a_alias,
        "the parent reads as its alias: {lane}"
    );
    let other = listed.iter().find(|row| row["alias"] == "lane60").unwrap();
    assert_eq!(other["parent"], "peer", "{other}");

    // An alias filter finds the agents by name, in any letter case.
    let found = call(
        &world,
        "b",
        LIST_SESSION_AGENTS_TOOL,
        serde_json::json!({ "alias": "LANE8" }),
    )
    .await;
    let aliases = found["agents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["alias"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(aliases.len(), 11, "lane8 and lane80..lane89: {aliases:?}");
    assert!(aliases.iter().all(|alias| alias.starts_with("lane8")));

    // A cursor the list does not know is refused with the next step.
    let refused = world
        .router
        .dispatch_authenticated_runtime_tool_call(
            &world.tokens["b"],
            LIST_SESSION_AGENTS_TOOL,
            serde_json::json!({ "cursor": "nobody" }),
        )
        .await
        .unwrap();
    assert!(
        !refused.ok && error_text(&refused).contains("without a cursor"),
        "{refused:?}"
    );
}

#[test]
fn room_overview_pages_its_agents_and_leaves_task_history_out_by_default() {
    run_large_stack_async_test("agent-pages-overview", overview_pages);
}

async fn overview_pages() {
    // The overview lists the Room's visible agents; the 90 sub-agents stay out of it, and
    // the task history that used to fill it is left out unless asked for.
    let world = crowded_world("agent-pages-overview").await;
    let page = call(
        &world,
        "a",
        META_SESSION_OVERVIEW_TOOL,
        serde_json::json!({}),
    )
    .await;
    assert!(size(&page) < 16 * 1024, "{} bytes", size(&page));
    assert!(page.get("agent_tasks").is_none() && page.get("activity").is_none());
    let visible = page["total"].as_u64().unwrap() as usize;
    assert!(visible >= 2, "{page}");
    let one = call(
        &world,
        "a",
        META_SESSION_OVERVIEW_TOOL,
        serde_json::json!({ "limit": 1 }),
    )
    .await;
    assert_eq!(one["agents"].as_array().unwrap().len(), 1);
    let cursor = one["next_cursor"].clone();
    assert!(cursor.is_string(), "{one}");
    let next = call(
        &world,
        "a",
        META_SESSION_OVERVIEW_TOOL,
        serde_json::json!({ "cursor": cursor, "limit": 200 }),
    )
    .await;
    assert_eq!(1 + next["agents"].as_array().unwrap().len(), visible);
    let with_tasks = call(
        &world,
        "a",
        META_SESSION_OVERVIEW_TOOL,
        serde_json::json!({ "include_tasks": true }),
    )
    .await;
    assert!(with_tasks["agent_tasks"].is_array());
}

#[test]
fn a_row_says_in_turn_between_turns_or_finished() {
    run_large_stack_async_test("agent-pages-turn", turn_states);
}

async fn turn_states() {
    let world = world("agent-pages-turn").await;
    let sent = room_command(&world, "a", "prompt b read the README").await;
    assert!(sent.ok, "{sent:?}");
    yield_on_a_timer(&world, "s", crate::session::unix_epoch_ms() - 120_000).await;
    let page = call(&world, "a", LIST_SESSION_AGENTS_TOOL, serde_json::json!({})).await;
    let turn = |alias: &str| {
        page["agents"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["alias"] == alias)
            .unwrap()["turn"]
            .clone()
    };
    assert_eq!(turn("b"), "in_turn", "a prompt is being worked");
    assert_eq!(turn("s"), "between_turns", "it yielded on a timer");
    assert_eq!(turn("d"), "finished", "nothing runs or waits");
    let inspected = call(
        &world,
        "a",
        GET_SESSION_AGENT_TOOL,
        serde_json::json!({ "agent": "s" }),
    )
    .await;
    assert_eq!(inspected["agent"]["turn"], "between_turns");
}
