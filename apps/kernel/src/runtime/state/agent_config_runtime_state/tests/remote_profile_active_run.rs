// MP-08 / MP-10 / MP-11: confirmed profiles retire only their cached leased runs.
use super::*;
use crate::provider::{ProviderRunState, RuntimeProviderRun};

enum SelectedRun {
    PreviousLease,
    OtherLease,
    Local,
}

async fn confirmed_remote_profile(
    selected: SelectedRun,
) -> (KernelRuntimeState, String, RuntimeProviderRun, String) {
    let (app, runtime, session_id, agent_id) = agent_config_runtime().await;
    let old_id =
        crate::provider::projected_leased_provider_run_id("leased-agent-1", "worker-run-old");
    let mut old = RuntimeProviderRun::from_control_capability_inference(
        "worker-run-old",
        "worker-session".into(),
        Some("leased-agent-1".into()),
        "dev-stub".into(),
    );
    // A completed worker turn can leave a running home projection, with no active worker turn.
    old.mark_running();
    let old = old.projected_for_home_agent_with_id(&old_id, &session_id, &agent_id);
    runtime
        .owned
        .provider_run_projection
        .update_remote_snapshot(old.clone());
    let active_id = {
        let mut app = app.lock().await;
        app.agents_mut()
            .bind_remote_execution(
                &agent_id,
                crate::agent::RemoteAgentBinding {
                    worker_kernel_id: "worker-1".into(),
                    worker_machine_id: "machine-1".into(),
                    execution_lease_id: "lease-1".into(),
                    leased_agent_id: "leased-agent-1".into(),
                    active_worker_provider_run_id: None,
                    relay_url: None,
                    relay_token: None,
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
                },
            )
            .unwrap();
        match selected {
            SelectedRun::PreviousLease => old_id,
            SelectedRun::OtherLease | SelectedRun::Local => {
                let other = app
                    .spawn_agent(crate::agent::CreateAgentRequest::new(
                        &session_id,
                        "dev-stub",
                    ))
                    .unwrap();
                let id = if matches!(selected, SelectedRun::OtherLease) {
                    crate::provider::projected_leased_provider_run_id(
                        "leased-agent-2",
                        "worker-run-active",
                    )
                } else {
                    "provider-run-local".into()
                };
                // Starting runs remain selected while their normal launch is in progress.
                let other_run = RuntimeProviderRun::from_control_capability_inference(
                    &id,
                    session_id.clone(),
                    Some(other.id().into()),
                    "dev-stub".into(),
                );
                runtime.owned.provider_run_projection.update(other_run);
                id
            }
        }
    };
    runtime
        .owned
        .session_store
        .set_active_provider_run(&session_id, Some(active_id.clone()))
        .unwrap();
    assert_eq!(
        runtime
            .session_snapshot(&session_id)
            .await
            .unwrap()
            .active_provider_run_id(),
        Some(active_id.as_str()),
        "fixture must select a registered provider run before the profile update"
    );
    let agent = runtime
        .owned
        .commit_remote_agent_profile_update(
            &session_id,
            &agent_id,
            "dev-stub".into(),
            "default".into(),
            Some("model-next".into()),
            None,
        )
        .unwrap();
    assert_eq!(agent.model(), Some("model-next"));
    assert_eq!(
        agent
            .remote_execution()
            .unwrap()
            .active_worker_provider_run_id,
        None
    );
    (runtime, session_id, old, active_id)
}

#[tokio::test]
async fn confirmed_remote_profile_retires_the_previous_leased_projection() {
    let (runtime, session_id, old, _) = confirmed_remote_profile(SelectedRun::PreviousLease).await;
    assert_eq!(
        runtime
            .session_snapshot(&session_id)
            .await
            .unwrap()
            .active_provider_run_id(),
        None
    );
    assert_eq!(
        runtime
            .owned
            .provider_run_projection
            .get(old.id())
            .unwrap()
            .state(),
        ProviderRunState::Ended
    );
    // A lagging worker snapshot must not advertise the replaced profile again.
    assert_eq!(
        runtime
            .owned
            .provider_run_projection
            .update_remote_snapshot(old)
            .state(),
        ProviderRunState::Ended
    );
    assert_eq!(
        runtime
            .session_snapshot(&session_id)
            .await
            .unwrap()
            .active_provider_run_id(),
        None
    );
}

#[tokio::test]
async fn confirmed_remote_profile_preserves_another_agents_active_run() {
    for selected in [SelectedRun::OtherLease, SelectedRun::Local] {
        let (runtime, session_id, _, active_id) = confirmed_remote_profile(selected).await;
        assert_eq!(
            runtime
                .session_snapshot(&session_id)
                .await
                .unwrap()
                .active_provider_run_id(),
            Some(active_id.as_str())
        );
        assert_eq!(
            runtime
                .owned
                .provider_run_projection
                .get(&active_id)
                .unwrap()
                .state(),
            ProviderRunState::Starting
        );
    }
}
