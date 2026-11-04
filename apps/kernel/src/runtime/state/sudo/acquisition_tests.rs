//! MP-08 / MP-09 / MP-10 / MP-11: only the caller's covered live sudo skips acquisition approval.
use super::tests::{fixture_with_options, running, Fixture};
use super::*;
use crate::provider::{
    AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult, RuntimeProviderRun,
};
use serde_json::{json, Value};

const TOKEN: &str = "sudo-fixture-bearer";

fn acquisition(action: &str) -> Value {
    let mut request =
        json!({"action": action, "capability_revision": "fixture", "request_id": "one"});
    if action != "acquire" {
        request["allocation_ref"] = "fixture-machine".into();
        request["expected_revision"] = 1.into();
    }
    if action == "start_saved_state" {
        request["saved_state_ref"] = "fixture-saved".into();
    }
    json!({"request": {"ControlPlane": request}})
}

#[tokio::test]
async fn default_on_machine_acquisition_uses_only_the_callers_covered_live_sudo_window() {
    for action in ["acquire", "start", "start_saved_state"] {
        let f = fixture_with_options(None, true);
        let turn = running(&f);
        assert!(f
            .state
            .owned
            .config_projection
            .snapshot()
            .user_config
            .agents
            .ask_before_acquiring_machines
            .unwrap_or(true));
        let caller = f.state.reach_for_auth_token(TOKEN).unwrap();
        assert_eq!(
            f.state.reach_sudo_turn(&caller).unwrap().entry_id,
            turn.entry_id
        );
        let request = acquisition(action);
        let typed: LocalDaemonRequest = serde_json::from_value(request["request"].clone()).unwrap();
        f.state
            .authorize_sudo_request(&turn.entry_id, &typed)
            .unwrap();
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            f.router.dispatch_reach_tool_call(TOKEN, "request", request),
        )
        .await
        .expect("a covered acquisition reaches the service without another human answer");
        // This real self-hosted control-plane entry has no service: reaching its definitive
        // error proves approval was bypassed, rather than discovery refusing the request first.
        assert!(
            matches!(result, Err(DaemonError::RelayTransport { code, .. }) if code == "control_plane_unavailable"),
            "{action} did not reach the authorized service operation"
        );
        assert!(f
            .state
            .owned
            .session_store
            .get_session(&turn.session_id)
            .unwrap()
            .active_interactions()
            .is_empty());
    }
}

async fn assert_start_still_asks(f: &Fixture, token: &str, agent: &str) {
    let request = f
        .router
        .dispatch_reach_tool_call(token, "request", acquisition("start"));
    tokio::pin!(request);
    let card = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            tokio::select! {
                result = &mut request => panic!("uncovered acquisition ran without a decision: {result:?}"),
                _ = tokio::time::sleep(Duration::from_millis(10)) => {}
            }
            let session = f.state.owned.session_store.get_session(&f.request.session_id).unwrap();
            if let Some(card) = session.active_interaction_for_agent(agent) { break card.id().to_owned(); }
        }
    }).await.expect("uncovered acquisition asks its own owner");
    f.state
        .timeout_runtime_interaction(&f.request.session_id, &card)
        .await
        .unwrap();
    let result = request.await.unwrap();
    assert!(!result.ok);
    assert_eq!(result.payload["code"], "acquisition_declined");
}

#[tokio::test]
async fn another_agents_live_sudo_window_does_not_approve_an_acquisition() {
    let f = fixture_with_options(None, true);
    let turn = running(&f);
    let peer = {
        let mut app = f.app.lock().await;
        let peer = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(crate::agent::CreateAgentRequest::new(
                &turn.session_id,
                "dev-stub",
            ))
            .unwrap();
        let launch = LaunchProviderRequest::new(
            &turn.session_id,
            "dev-stub",
            "dev-stub",
            "default",
            "default",
        )
        .with_agent_id(peer.id());
        let mut run = RuntimeProviderRun::new(
            "sudo-acquisition-peer-run",
            &launch,
            ProviderLaunchResult {
                endpoint_mode: AgentEndpointMode::External,
                process_label: "metadata-only".into(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                pty_env: Default::default(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: None,
            },
        );
        run.mark_running();
        run.set_runtime_mcp_auth_token(Some("sudo-acquisition-peer-bearer".into()));
        app.providers_mut().insert_run_for_test(run);
        peer
    };
    let caller = f
        .state
        .reach_for_auth_token("sudo-acquisition-peer-bearer")
        .unwrap();
    assert!(f.state.reach_sudo_turn(&caller).is_none());
    assert_start_still_asks(&f, "sudo-acquisition-peer-bearer", peer.id()).await;
    assert!(
        f.state.sudo_for_auth_token(TOKEN).is_ok(),
        "the original agent retains its own live window"
    );
}

#[tokio::test]
async fn an_expired_sudo_window_does_not_approve_an_acquisition() {
    let f = fixture_with_options(None, true);
    let turn = running(&f);
    {
        let mut turns = f.state.owned.sudo_turns.lock().unwrap();
        let stale = turns.get_mut(&turn.entry_id).unwrap();
        stale.deadline = Some(std::time::Instant::now() - Duration::from_secs(1));
        stale.expires_at_ms = Some(0);
    }
    assert_start_still_asks(&f, TOKEN, &turn.agent_id).await;
}
