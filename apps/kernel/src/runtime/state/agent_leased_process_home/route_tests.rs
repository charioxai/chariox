//! MP-08/MP-09/MP-10/MP-11 A10: a watched-process request reaches a leased worker over the connected relay.
use super::*;
use crate::runtime::state::leased_worker_route_tests::{leased_home, LEASED_AGENT, WORKER_RUN};

#[tokio::test]
async fn watched_process_query_goes_over_the_connected_relay_that_reaches_the_worker() {
    let mut home = leased_home("leased-route-process").await;
    let wake: AgentWake = serde_json::from_value(serde_json::json!({
        "id": "wake", "task_id": "task", "room_id": home.session_id, "agent_id": home.agent_id,
        "registration_id": "completion-wake", "kind": "process", "label": "watch",
        "state": "running", "created_at_ms": 1, "verified_at_ms": null, "next_due_ms": null,
        "interval_ms": null, "command": ["true"], "match_text": null, "matched_at_ms": null,
        "pid": null, "exit_code": null, "fire_count": 0, "missed_fires": 0,
        "last_fired_at_ms": null, "last_sequence": null, "last_delivery": null,
        "last_delivered_at_ms": null, "last_acknowledged_at_ms": null, "alerted_sequence": null
    }))
    .unwrap();
    let binding = LeasedProcessBinding {
        worker_kernel_id: home.rig.worker_kernel_id().into(),
        worker_machine_id: "machine".into(),
        execution_lease_id: "lease".into(),
        leased_agent_id: LEASED_AGENT.into(),
        home_prompt_id: "home-prompt".into(),
        worker_provider_run_id: WORKER_RUN.into(),
    };
    let action = Action::Query {
        wake_id: wake.id.clone(),
        binding: binding.clone(),
    };
    let call = home.state.leased_process_request(&wake, &binding, action);
    let reply = home
        .rig
        .answer(|_| RelayPeerResponse::LeasedWatchedProcess { receipt: None });
    let (result, request) = tokio::join!(call, reply);
    assert!(result.expect("the worker answers the query").is_none());
    assert!(matches!(
        request,
        RelayPeerRequest::LeasedWatchedProcess { leased_agent_id, .. } if leased_agent_id == LEASED_AGENT
    ));
}
