//! A leased agent's settings changed while its turn runs: the worker takes them at its own step
//! boundary, in a new provider process, and the same turn goes on.

use super::*;
use crate::provider::{AgentExecutionMode, AgentPermissionLevel, ProviderRunState};
use crate::runtime::router::CommandRouter;

const NEXT_MODEL: &str = "next-model";

/// A worker whose one leased agent is working on a turn: the provider has the request.
struct RunningLeasedTurn {
    runtime: KernelRuntimeState,
    leased: crate::execution_lease::LeasedAgent,
    run_id: String,
    prompt_id: String,
}

async fn running_leased_turn() -> RunningLeasedTurn {
    let mut config = crate::config::DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = crate::app::DaemonApp::bootstrap(config).unwrap();
    let lease = crate::app::RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease(
            "home-kernel",
            "home-session",
            "home-agent",
            false,
            crate::session::DEFAULT_LOCAL_USER_ID,
        )
        .unwrap();
    let leased = crate::app::RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &lease.id,
            "managed-dev-stub",
            "default",
            Some("first-model".into()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
    // The worker's provider run is ready, so the turn is admitted at once.
    let prepared = crate::app::RemoteLeaseRuntime::new(&mut app)
        .prepare_leased_prompt_submission(
            &leased.id,
            "Work on the request",
            "",
            Vec::new(),
            None,
            None,
            Vec::new(),
            crate::extension::RemoteExtensionManifest::default(),
            crate::transport::relay_peer::LeasedNativeSkills::default(),
        )
        .unwrap();
    if let crate::app::PreparedLeasedProviderRun::LaunchRequired(request) = prepared.provider_run {
        app.launch_leased_provider(request).unwrap();
    }
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    let runtime = router.runtime_state();
    let (run_id, _) = runtime
        .submit_relay_leased_prompt(
            &leased.id,
            crate::transport::relay_peer::RelayAgentExecutionProfile {
                provider: leased.provider.clone(),
                account_profile: leased.account_profile.clone(),
                model: leased.model.clone(),
                effort: leased.effort.clone(),
            },
            None,
            "Work on the request",
            "",
            Vec::new(),
            None,
            None,
            Vec::new(),
            crate::extension::RemoteExtensionManifest::default(),
            None,
            crate::transport::relay_peer::LeasedNativeSkills::default(),
        )
        .await
        .expect("the worker admits the turn");
    let session = runtime
        .owned
        .session_store
        .get_session(&leased.backing_session_id)
        .unwrap();
    let prompt_id = runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &leased.backing_agent_id)
        .expect("the turn is active")
        .id()
        .to_string();
    runtime
        .owned
        .mark_active_prompt_delivery(
            &leased.backing_session_id,
            &leased.backing_agent_id,
            &prompt_id,
            crate::session::DurablePromptDeliveryPhase::Delivered,
            Some(run_id.clone()),
            None,
        )
        .expect("delivery is marked");
    runtime
        .owned
        .prompt_activity
        .write()
        .entry(run_id.clone())
        .or_insert(crate::app::ActivePromptState {
            last_output_at: None,
            saw_response_content: false,
            completion_recorded: false,
            settlement_requested: false,
            active_tools: Default::default(),
            active_tool_started_at_ms: Default::default(),
        });
    RunningLeasedTurn {
        runtime,
        leased,
        run_id,
        prompt_id,
    }
}

impl RunningLeasedTurn {
    fn live_run(&self) -> crate::provider::RuntimeProviderRun {
        self.runtime
            .owned
            .provider_store
            .get_run_for_agent(
                &self.leased.backing_session_id,
                &self.leased.backing_agent_id,
            )
            .expect("the agent has a provider run")
    }

    fn active_prompt_id(&self) -> Option<String> {
        let session = self
            .runtime
            .owned
            .session_store
            .get_session(&self.leased.backing_session_id)
            .unwrap();
        self.runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &self.leased.backing_agent_id)
            .map(|prompt| prompt.id().to_string())
    }
}

#[tokio::test]
async fn a_leased_turn_goes_on_in_a_new_process_when_its_model_changes() {
    let turn = running_leased_turn().await;

    let updated = turn
        .runtime
        .update_relay_leased_agent_profile(
            &turn.leased.id,
            "managed-dev-stub".into(),
            "default".into(),
            Some(NEXT_MODEL.into()),
            None,
        )
        .await
        .expect("a leased agent accepts a profile change while its turn runs");

    assert_eq!(updated.model.as_deref(), Some(NEXT_MODEL));
    let old_run = turn.live_run();
    assert_eq!(old_run.id(), turn.run_id, "the process keeps working");
    assert!(old_run.settings_stale());

    let swapped = turn
        .runtime
        .swap_turn_process_at_step_boundary(&turn.leased.backing_session_id, &turn.run_id)
        .await
        .expect("the swap succeeds");

    assert!(swapped);
    assert_eq!(
        turn.runtime
            .owned
            .provider_store
            .get_run(&turn.run_id)
            .unwrap()
            .state(),
        ProviderRunState::Ended
    );
    let new_run = turn.live_run();
    assert_ne!(new_run.id(), turn.run_id);
    assert_eq!(new_run.model(), NEXT_MODEL);
    assert_eq!(
        turn.active_prompt_id().as_deref(),
        Some(turn.prompt_id.as_str())
    );
}

#[tokio::test]
async fn a_leased_turn_takes_a_mode_change_without_ending() {
    let turn = running_leased_turn().await;

    let updated = turn
        .runtime
        .update_relay_leased_agent_config(
            &turn.leased.id,
            AgentExecutionMode::Plan,
            AgentPermissionLevel::Required,
        )
        .await
        .expect("a leased agent accepts a config change while its turn runs");

    assert_eq!(updated.execution_mode, Some(AgentExecutionMode::Plan));
    assert_eq!(
        updated.permission_level,
        Some(AgentPermissionLevel::Required)
    );
    let run = turn.live_run();
    assert_eq!(run.id(), turn.run_id, "the turn is not ended");
    assert_ne!(run.state(), ProviderRunState::Ended);
    assert_eq!(
        turn.active_prompt_id().as_deref(),
        Some(turn.prompt_id.as_str())
    );
}
