//! MP-08/MP-10/MP-11 A04: git observations after a leased turn are harvested
//! over the relay connection that already reaches the worker. A hosted or
//! slice worker is not visible to a temporary connection's metadata lookup.
use super::*;
use crate::app::{DaemonApp, KernelSessionService, RemoteLeaseRuntime};
use crate::history::HistoryEventKind;
use crate::transport::relay_client::ConnectedWorkerRig;
use crate::transport::relay_peer::RemoteGitObservation;

#[tokio::test(flavor = "multi_thread")]
async fn harvest_goes_over_the_connected_relay_that_reaches_the_worker() {
    let workspace = crate::test_support::TestWorktree::new("leased-route-harvest");
    let mut config = crate::config::DaemonConfig::for_tests();
    config.relay_url = Some("ws://fixture.invalid".into());
    config.relay_token = Some("home-token".into());
    let mut app = DaemonApp::bootstrap(config.clone()).unwrap();
    let (session, agent) = KernelSessionService::new(&mut app)
        .create_session(workspace.session_request())
        .unwrap();
    let mut rig =
        ConnectedWorkerRig::connect(&app.relay_client_state(), &config, "ws://fixture.invalid")
            .await;
    app.agents()
        .bind_remote_execution(
            agent.id(),
            serde_json::from_value(serde_json::json!({
                "worker_kernel_id": rig.worker_kernel_id(), "worker_machine_id": "machine",
                "execution_lease_id": "lease", "leased_agent_id": "leased",
                "active_worker_provider_run_id": "worker-run",
                "relay_peer_protocol_version": crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION
            }))
            .unwrap(),
        )
        .unwrap();
    let remote = app
        .agents
        .get_agent(agent.id())
        .unwrap()
        .remote_execution()
        .cloned()
        .unwrap();
    let observation = RemoteGitObservation {
        kind: HistoryEventKind::GitCommitDetected,
        content: Some("slice commit".into()),
        metadata: Default::default(),
        context: crate::history::HistoryEventTurnContext {
            session_id: Some(session.id().to_string()),
            agent_id: Some(agent.id().to_string()),
            ..Default::default()
        },
        candidate_agent_ids: vec![agent.id().to_string()],
        candidate_prompt_ids: vec![],
        candidate_turn_ids: vec![],
        attribution_confidence: None,
    };
    let reply = tokio::spawn(async move {
        let request = rig
            .answer(|_| RelayPeerResponse::LeasedGitObserved {
                provider_run_id: "worker-run".into(),
                git_observations: vec![observation],
                workspace_live_sync_change: None,
            })
            .await;
        (rig, request)
    });
    RemoteLeaseRuntime::new(&mut app).harvest_remote_completion_observations(&remote, "worker-run");
    let (_rig, request) = reply.await.unwrap();
    assert!(matches!(
        request,
        RelayPeerRequest::ObserveLeasedGitAfter { leased_agent_id, provider_run_id }
            if leased_agent_id == "leased" && provider_run_id == "worker-run"
    ));
    let events = app
        .operational_history_store()
        .query_events(crate::history::HistoryEventQuery {
            session_id: Some(session.id().to_string()),
            ..Default::default()
        })
        .unwrap();
    assert!(
        events
            .iter()
            .any(|event| event.content.as_deref() == Some("slice commit")),
        "the worker's git observation reaches the home history"
    );
}
