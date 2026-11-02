//! MP-08/MP-09/MP-10/MP-11 A04: a leased sudo fence takes the route plain prompts take.
use super::*;
use crate::runtime::router::CommandRouter;
use crate::transport::relay_peer::{LeasedSudoGrant, RelayPeerRequest, RelayPeerResponse};
use crate::transport::{relay_client, relay_crypto};
use chariox_relay::protocol::RelayEnvelope;

/// A slice or hosted worker is not visible to a temporary connection's relay
/// metadata lookup; only the connected relay holds its identity and peer key.
/// Base: the fence opened a temporary connection and failed discovery.
#[tokio::test]
async fn leased_sudo_fence_goes_over_the_connected_relay_that_reaches_the_worker() {
    let workspace = crate::test_support::TestWorktree::new("leased-sudo-route");
    let mut config = crate::config::DaemonConfig::for_tests();
    config.relay_url = Some("ws://fixture.invalid".into());
    config.relay_token = Some("home-token".into());
    let worker = crate::config::DaemonConfig::for_tests();
    let mut app = crate::app::DaemonApp::bootstrap(config.clone()).unwrap();
    let (_session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(workspace.session_request())
        .unwrap();
    app.agents()
        .bind_remote_execution(
            agent.id(),
            serde_json::from_value(serde_json::json!({
                "worker_kernel_id": worker.daemon_id, "worker_machine_id":"machine",
                "execution_lease_id":"lease", "leased_agent_id":"leased",
                "active_worker_provider_run_id":"run", "relay_peer_protocol_version":109
            }))
            .unwrap(),
        )
        .unwrap();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    let state = router.runtime_state();
    let (sender, mut wire, _events) = relay_client::RelayOutgoingSender::channel(4);
    {
        let mut relay = state.owned.relay_state.write().await;
        relay.test_set_connected_sender(sender, "ws://fixture.invalid");
        relay.remember_peer_public_key(&worker.daemon_id, worker.relay_public_key.clone());
    }
    let grant = LeasedSudoGrant {
        entry_id: "entry".into(),
        revision: 1,
        remaining_ms: 60_000,
        initial: true,
    };
    let call = state.push_leased_sudo(agent.id(), "home-prompt", grant.clone());
    let reply = async {
        let envelope = tokio::time::timeout(Duration::from_secs(5), wire.recv())
            .await
            .expect("the fence must use the connected encrypted peer transport")
            .unwrap();
        let RelayEnvelope::DaemonPeerRequest {
            request_id,
            target,
            encrypted_request,
        } = envelope
        else {
            panic!("expected a peer request")
        };
        assert_eq!(target.daemon_id.as_deref(), Some(worker.daemon_id.as_str()));
        let decoded = relay_crypto::decrypt_payload_for_private_key(
            &worker.relay_private_key,
            &encrypted_request,
        )
        .unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&decoded.plaintext).unwrap(),
            serde_json::to_value(RelayPeerRequest::UpdateLeasedSudo {
                leased_agent_id: "leased".into(),
                home_prompt_id: "home-prompt".into(),
                grant,
            })
            .unwrap()
        );
        let encrypted = relay_crypto::encrypt_payload_for_peer(
            &worker.relay_private_key,
            &config.relay_public_key,
            &serde_json::to_vec(&RelayPeerResponse::LeasedSudoUpdated).unwrap(),
        )
        .unwrap();
        relay_client::resolve_pending_peer_response_for_test(
            &state.owned.relay_state,
            request_id,
            worker.daemon_id.clone(),
            encrypted,
        )
        .await;
    };
    let (result, ()) = tokio::join!(call, reply);
    result.expect("the worker confirms the fence");
    state.shutdown_cleanup().await.unwrap();
}
