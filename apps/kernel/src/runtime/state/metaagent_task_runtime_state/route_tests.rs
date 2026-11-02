//! MP-08/MP-10/MP-11 A04: the meta-mode update reaches a leased worker over the connected relay.
use crate::runtime::state::leased_worker_route_tests::{leased_home, LEASED_AGENT};
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};

#[tokio::test]
async fn meta_mode_sync_goes_over_the_connected_relay_that_reaches_the_worker() {
    let mut home = leased_home("leased-route-meta-mode").await;
    let call =
        home.state
            .sync_remote_leased_agent_meta_mode(&home.session_id, &home.agent_id, true);
    let reply = home
        .rig
        .answer(|_| RelayPeerResponse::LeasedAgentMetaModeUpdated {
            leased_agent: serde_json::from_value(serde_json::json!({
                "id": LEASED_AGENT, "lease_id": "lease", "home_agent_id": "home-agent",
                "provider": "codex", "account_profile": "default", "model": null, "effort": null,
                "execution_mode": null, "permission_level": null, "backing_session_id": "backing",
                "backing_agent_id": "backing-agent", "backing_attachment_id": "backing-attachment",
                "created_at_ms": 1
            }))
            .unwrap(),
        });
    let (result, request) = tokio::join!(call, reply);
    result.expect("the worker confirms the meta mode");
    assert!(matches!(
        request,
        RelayPeerRequest::UpdateLeasedAgentMetaMode { leased_agent_id, active: true }
            if leased_agent_id == LEASED_AGENT
    ));
}
