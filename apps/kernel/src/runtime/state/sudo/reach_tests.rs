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

// MP-08 / MP-09 / MP-10 / MP-11: enrollment does not strand the owner's older rooms.
#[tokio::test]
async fn ordinary_reach_reads_pre_enrollment_rooms_as_their_member() {
    let f =
        super::tests::fixture_for_owner(None, true, "dev-stub", "dev-stub", None, "enrolled-owner");
    let root = crate::test_support::TestWorktree::new("reach-before-enrollment");
    let (legacy, agent) = crate::app::KernelSessionService::new(&mut *f.app.lock().await)
        .create_session(root.session_request())
        .unwrap();
    assert_eq!(legacy.owner_user_id(), "local");
    let mut config = f.state.owned.config_projection.snapshot();
    config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
        user_id: "enrolled-owner".into(),
        ..Default::default()
    });
    f.state.owned.config_projection.update(config);
    let caller = f.state.reach_for_auth_token("sudo-fixture-bearer").unwrap();
    assert_eq!(caller.owner_user_id, "enrolled-owner");
    for request in [
        json!({"GetSessionState": {"session_id": legacy.id()}}),
        json!({"ListAgents": {"session_id": legacy.id()}}),
    ] {
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
        let agents = if result.payload.get("SessionState").is_some() {
            &result.payload["SessionState"]["session"]["agents"]
        } else {
            &result.payload["AgentsListed"]["agents"]
        };
        assert!(
            agents
                .as_array()
                .unwrap()
                .iter()
                .any(|entry| entry["id"] == agent.id()),
            "the local member's agent is visible: {result:?}"
        );
    }
    let result = f.router.dispatch_reach_tool_call("sudo-fixture-bearer", "request", json!({"request": {"AttachToSession": {
        "session_id": legacy.id(), "client_id": "reach-enrolled", "capability_level": "FullTerminal"
    }}})).await.unwrap();
    assert!(result.ok, "{result:?}");
    assert_eq!(
        f.state
            .owned
            .attachment_store
            .get_attachment(
                result.payload["SessionAttached"]["attachment"]["id"]
                    .as_str()
                    .unwrap()
            )
            .unwrap()
            .owner_user_id(),
        "local"
    );
    let subscribed = f
        .router
        .dispatch_reach_tool_call(
            "sudo-fixture-bearer",
            "events_subscribe",
            json!({"kernel": "local", "session_id": legacy.id(), "agent_id": agent.id()}),
        )
        .await
        .unwrap();
    assert!(subscribed.ok, "{subscribed:?}");
    f.state.start_active_turn_with_trace_id(
        legacy.id(),
        agent.id(),
        "legacy-prompt",
        "legacy-run",
        "legacy-trace",
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
            .any(|event| event["agent_id"] == agent.id()),
        "{waited:?}"
    );
    // Membership/view mapping must not replace the authenticated identity or widen authority.
    assert_eq!(
        f.state
            .reach_for_auth_token("sudo-fixture-bearer")
            .unwrap()
            .owner_user_id,
        "enrolled-owner"
    );
    let result = f
        .router
        .dispatch_reach_tool_call(
            "sudo-fixture-bearer",
            "request",
            json!({"request": {"UpdateAgentProfile": {
                "session_id": legacy.id(), "agent_id": agent.id(), "model": "changed"
            }}}),
        )
        .await
        .unwrap();
    assert!(!result.ok);
    assert_eq!(result.payload["code"], "reach_requires_sudo");
    let (other, _) = crate::app::KernelSessionService::new(&mut *f.app.lock().await)
        .create_session(root.session_request().with_owner_user_id("stranger"))
        .unwrap();
    let result = f
        .router
        .dispatch_reach_tool_call(
            "sudo-fixture-bearer",
            "request",
            json!({"request": {"GetSessionState": {"session_id": other.id()}}}),
        )
        .await
        .unwrap();
    assert!(
        !result.ok,
        "the owner's mapping does not admit another user's room"
    );
    let refused = f
        .router
        .dispatch_reach_tool_call(
            "sudo-fixture-bearer",
            "events_subscribe",
            json!({"kernel": "local", "session_id": other.id()}),
        )
        .await
        .unwrap();
    assert!(!refused.ok, "{refused:?}");
    assert_eq!(refused.payload["code"], "events_refused");
}
