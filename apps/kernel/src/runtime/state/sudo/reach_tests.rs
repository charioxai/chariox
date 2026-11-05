//! MP-08 / MP-09 / MP-10 / MP-11: sudo authorizes creation; reach owns its events.
use super::tests::{fixture, running};
use serde_json::json;

#[tokio::test]
async fn elevated_reach_spawns_follow_the_reach_inbox() {
    for batch in [false, true] {
        for no_subscribe in [false, true] {
            let f = fixture();
            let turn = running(&f);
            let child = json!({"provider": "dev-stub", "alias": "delegate"});
            let request = if batch {
                json!({"SpawnAgents": {"session_id": turn.session_id,
                    "agents": [child], "no_subscribe": no_subscribe}})
            } else {
                let mut child = child;
                child["session_id"] = turn.session_id.clone().into();
                child["no_subscribe"] = no_subscribe.into();
                json!({"SpawnAgent": child})
            };
            let result = f
                .router
                .dispatch_reach_tool_call(
                    "sudo-fixture-bearer",
                    "request",
                    json!({"request": request}),
                )
                .await
                .unwrap();
            assert!(result.ok, "{result:?}");
            let created = if batch {
                &result.payload["AgentsSpawned"]
            } else {
                &result.payload["AgentSpawned"]
            };
            assert!(created["event_subscription_error"].is_null(), "{created}");
            let agent = if batch {
                &created["agents"][0]
            } else {
                &created["agent"]
            };
            let agent_id = agent["id"].as_str().expect("spawned agent");
            let caller = f.state.reach_for_auth_token("sudo-fixture-bearer").unwrap();
            let part = f.state.outside_store().outside_part(&caller.id).unwrap();
            assert_eq!(
                part.as_ref().is_some_and(|part| part
                    .targets
                    .iter()
                    .any(|target| target.agent_id.as_deref() == Some(agent_id))),
                !no_subscribe
            );
            assert!(f
                .state
                .outside_store()
                .outside_part(&turn.entry_id)
                .unwrap()
                .is_none());
            if no_subscribe {
                continue;
            }
            f.state.start_active_turn_with_trace_id(
                &turn.session_id,
                agent_id,
                "delegate-prompt",
                "delegate-run",
                "delegate-trace",
            );
            let waited = f
                .router
                .dispatch_reach_tool_call(
                    "sudo-fixture-bearer",
                    "events_wait",
                    json!({"timeout_seconds": 1}),
                )
                .await
                .unwrap();
            assert!(waited.ok, "{waited:?}");
            assert!(
                waited.payload["EventsListed"]["events"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|event| event["agent_id"] == agent_id),
                "{waited:?}"
            );
        }
    }
}
