use super::*;
use crate::local::AgentSubstituteAction;
use crate::provider::{LaunchProviderRequest, ProviderRunState};

async fn configured_runtime() -> (Arc<Mutex<DaemonApp>>, KernelRuntimeState, String, String) {
    let (app, runtime, session, agent) = agent_config_runtime().await;
    runtime
        .update_agent_profile(
            &session,
            &agent,
            crate::session::DEFAULT_LOCAL_USER_ID,
            Some("dev-stub".into()),
            Some("starter-account".into()),
            Some("starter-model".into()),
            Some(Some("high".into())),
        )
        .await
        .unwrap();
    runtime
        .update_agent_substitutes(
            &session,
            &agent,
            crate::session::DEFAULT_LOCAL_USER_ID,
            AgentSubstituteAction::Add {
                provider: "dev-stub".into(),
                model: "substitute-model".into(),
                variant: Some("low".into()),
                account_profile: Some("substitute-account".into()),
                kernel_id: None,
                worktree_id: None,
            },
        )
        .await
        .unwrap();
    (app, runtime, session, agent)
}

fn start_stub(runtime: &KernelRuntimeState, session: &str, agent: &str) -> String {
    let config = runtime.owned.agent_store.get_agent(agent).unwrap();
    let run = runtime
        .owned
        .provider_store
        .launch_run_detached(
            LaunchProviderRequest::new(
                session,
                "dev-stub",
                config.provider(),
                config.provider_account_profile(),
                config.model().unwrap(),
            )
            .with_agent_id(agent)
            .with_variant(config.effort().map(str::to_string)),
        )
        .unwrap();
    runtime
        .owned
        .session_store
        .set_active_provider_run(session, Some(run.id().into()))
        .unwrap();
    run.id().into()
}

#[tokio::test]
async fn manual_substitute_activation_and_reset_retire_old_runs() {
    let (app, runtime, session, agent) = configured_runtime().await;
    for action in [
        AgentSubstituteAction::Activate {
            index: 0,
            reason: None,
        },
        AgentSubstituteAction::Primary {},
    ] {
        let old = start_stub(&runtime, &session, &agent);
        runtime
            .update_agent_substitutes(
                &session,
                &agent,
                crate::session::DEFAULT_LOCAL_USER_ID,
                action,
            )
            .await
            .unwrap();
        assert_eq!(
            runtime.owned.provider_store.get_run(&old).unwrap().state(),
            ProviderRunState::Ended,
            "selection changes must retire the previous execution identity"
        );
        assert!(runtime
            .owned
            .session_store
            .get_session(&session)
            .unwrap()
            .active_provider_run_id()
            .is_none());
    }
    let selected = runtime.owned.agent_store.get_agent(&agent).unwrap();
    assert_eq!(selected.model(), Some("starter-model"));
    assert_eq!(selected.provider_account_profile(), "starter-account");
    let next = app
        .lock()
        .await
        .ensure_prompt_provider_run_for_agent(&session, &agent)
        .unwrap();
    let run = runtime.owned.provider_store.get_run(&next).unwrap();
    assert_eq!(run.account_profile(), "starter-account");
    assert_eq!(run.variant(), Some("high"));
    let events = runtime
        .owned
        .durable_state_store
        .load_events_by_kind("agent.updated")
        .unwrap();
    assert!(events
        .iter()
        .any(
            |event| event.payload["agent"]["account_profile"] == "starter-account"
                && event.payload["agent"]["active_substitute_index"].is_null()
        ));
}

#[tokio::test]
async fn manual_substitute_changes_reject_active_turn_without_mutation() {
    for action in [
        AgentSubstituteAction::Activate {
            index: 0,
            reason: None,
        },
        AgentSubstituteAction::Primary {},
        AgentSubstituteAction::Remove { index: 0 },
        AgentSubstituteAction::Clear {},
    ] {
        let (app, runtime, session, agent) = configured_runtime().await;
        if !matches!(action, AgentSubstituteAction::Activate { .. }) {
            runtime
                .update_agent_substitutes(
                    &session,
                    &agent,
                    crate::session::DEFAULT_LOCAL_USER_ID,
                    AgentSubstituteAction::Activate {
                        index: 0,
                        reason: None,
                    },
                )
                .await
                .unwrap();
        }
        let old = start_stub(&runtime, &session, &agent);
        sync_active_prompt(&app, &session, &agent).await;
        let before = runtime.owned.agent_store.get_agent(&agent).unwrap();
        let error = runtime
            .update_agent_substitutes(
                &session,
                &agent,
                crate::session::DEFAULT_LOCAL_USER_ID,
                action,
            )
            .await
            .expect_err("manual switching cannot reassign an active turn");
        assert_active_turn_error(error, "update agent substitutes");
        assert_eq!(
            serde_json::to_value(runtime.owned.agent_store.get_agent(&agent).unwrap()).unwrap(),
            serde_json::to_value(before).unwrap()
        );
        assert_eq!(
            runtime.owned.provider_store.get_run(&old).unwrap().state(),
            ProviderRunState::Running
        );
    }
}

#[tokio::test]
async fn stale_processing_does_not_release_dispatching_prompt_for_substitute_change() {
    let (app, runtime, session, agent) = configured_runtime().await;
    let provider_run_id = start_stub(&runtime, &session, &agent);
    let prompt_id = "stale-dispatching-review";
    let mut prompt = crate::session::PromptQueueItem::new(
        prompt_id,
        "attachment-1",
        &agent,
        "review the current head",
        crate::session::PromptStatus::Dispatching,
    )
    .with_durable_operation("review-turn", "review-turn-fingerprint");
    prompt.set_durable_delivery(
        crate::session::DurablePromptDeliveryPhase::Dispatching,
        Some(provider_run_id.clone()),
        Some("provider-session-review".to_string()),
    );
    {
        let mut app = app.lock().await;
        app.prompt_owner_sync_external_active_prompt(&session, &agent, Some(prompt))
            .expect("the stale provider-owned prompt should be visible to the owner");
        app.agents_mut()
            .set_agent_processing(&agent, true)
            .expect("the legacy processing projection should be mutable");
        app.agents_mut()
            .set_agent_processing(&agent, false)
            .expect("the stale processing projection should be cleared");
        assert!(!app
            .agents()
            .get_agent(&agent)
            .expect("agent should remain available")
            .is_processing());
    }

    let started = crate::app::StartedProviderLaunch {
        run: runtime
            .owned
            .provider_store
            .get_run(&provider_run_id)
            .expect("provider run should remain addressable")
            .clone(),
        previous_active_run_id: None,
        provider_credential_env: Default::default(),
    };
    runtime
        .fail_provider_launch(
            &started,
            &DaemonError::LocalTransport {
                operation: "provider launch",
                message: "native reviewer launch stopped before readiness".to_string(),
            },
        )
        .await;
    assert_eq!(
        runtime
            .owned
            .provider_store
            .get_run(&provider_run_id)
            .expect("failed provider run should remain addressable")
            .state(),
        ProviderRunState::Ended,
    );

    let error = runtime
        .update_agent_substitutes(
            &session,
            &agent,
            crate::session::DEFAULT_LOCAL_USER_ID,
            AgentSubstituteAction::Activate {
                index: 0,
                reason: None,
            },
        )
        .await
        .expect_err("stale processing must not bypass the authoritative prompt owner");
    assert_active_turn_error(error, "update agent substitutes");
    assert_eq!(
        runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(
                &runtime.owned.session_store.get_session(&session).unwrap(),
                &agent,
            )
            .expect("the dispatching prompt should remain owned")
            .id(),
        prompt_id,
    );
    assert_eq!(
        runtime
            .owned
            .provider_store
            .get_run(&provider_run_id)
            .expect("provider run should remain addressable")
            .state(),
        ProviderRunState::Ended,
    );

    // The canonical prompt-owner cancellation seam is what resolves an
    // uncertain Dispatching turn. Only after it clears ownership may a
    // substitute switch retire the old provider run.
    app.lock()
        .await
        .prompt_owner_cancel_active_prompt_only(&session, &agent)
        .expect("canonical cancellation should clear the stale prompt owner");
    runtime
        .update_agent_substitutes(
            &session,
            &agent,
            crate::session::DEFAULT_LOCAL_USER_ID,
            AgentSubstituteAction::Activate {
                index: 0,
                reason: None,
            },
        )
        .await
        .expect("substitute activation should proceed after canonical cancellation");
    assert_eq!(
        runtime
            .owned
            .agent_store
            .get_agent(&agent)
            .unwrap()
            .active_substitute_index(),
        Some(0),
    );
    assert_eq!(
        runtime
            .owned
            .provider_store
            .get_run(&provider_run_id)
            .unwrap()
            .state(),
        ProviderRunState::Ended,
    );
}

#[tokio::test]
async fn missing_substitute_account_does_not_retire_or_mutate_starter() {
    let (_app, runtime, session, agent) = configured_runtime().await;
    runtime
        .owned
        .agent_store
        .add_agent_substitute(
            &agent,
            crate::agent::AgentSubstituteProfile::new(
                "claude-headless",
                "claude-opus-4-8",
                Some("high".into()),
            )
            .with_account_profile(Some("removed-private-account".into())),
        )
        .unwrap();
    let old = start_stub(&runtime, &session, &agent);
    let before = runtime.owned.agent_store.get_agent(&agent).unwrap();
    let error = runtime
        .update_agent_substitutes(
            &session,
            &agent,
            crate::session::DEFAULT_LOCAL_USER_ID,
            AgentSubstituteAction::Activate {
                index: 1,
                reason: None,
            },
        )
        .await
        .unwrap_err();
    assert!(!error.to_string().contains("removed-private-account"));
    assert_eq!(
        serde_json::to_value(runtime.owned.agent_store.get_agent(&agent).unwrap()).unwrap(),
        serde_json::to_value(before).unwrap()
    );
    assert_eq!(
        runtime.owned.provider_store.get_run(&old).unwrap().state(),
        ProviderRunState::Running
    );
}

#[tokio::test]
async fn inactive_substitute_edits_do_not_interrupt_the_running_profile() {
    let (app, runtime, session, agent) = configured_runtime().await;
    let old = start_stub(&runtime, &session, &agent);
    sync_active_prompt(&app, &session, &agent).await;
    for action in [
        AgentSubstituteAction::Move {
            from_index: 0,
            to_index: 0,
        },
        AgentSubstituteAction::SetTimeout {
            timeout_ms: Some(30_000),
        },
        AgentSubstituteAction::Primary {},
        AgentSubstituteAction::Remove { index: 0 },
    ] {
        runtime
            .update_agent_substitutes(
                &session,
                &agent,
                crate::session::DEFAULT_LOCAL_USER_ID,
                action,
            )
            .await
            .unwrap();
        assert_eq!(
            runtime.owned.provider_store.get_run(&old).unwrap().state(),
            ProviderRunState::Running
        );
    }
}

#[tokio::test]
async fn removing_or_clearing_active_substitute_retires_its_run() {
    for action in [
        AgentSubstituteAction::Remove { index: 0 },
        AgentSubstituteAction::Clear {},
    ] {
        let (_app, runtime, session, agent) = configured_runtime().await;
        runtime
            .update_agent_substitutes(
                &session,
                &agent,
                crate::session::DEFAULT_LOCAL_USER_ID,
                AgentSubstituteAction::Activate {
                    index: 0,
                    reason: None,
                },
            )
            .await
            .unwrap();
        let old = start_stub(&runtime, &session, &agent);
        runtime
            .update_agent_substitutes(
                &session,
                &agent,
                crate::session::DEFAULT_LOCAL_USER_ID,
                action,
            )
            .await
            .unwrap();
        assert_eq!(
            runtime.owned.provider_store.get_run(&old).unwrap().state(),
            ProviderRunState::Ended
        );
        assert_eq!(
            runtime
                .owned
                .agent_store
                .get_agent(&agent)
                .unwrap()
                .provider_account_profile(),
            "starter-account"
        );
    }
}

#[tokio::test]
async fn workflow_rotation_does_not_copy_another_providers_adapter() {
    for fresh in [false, true] {
        let (_app, runtime, session, agent) = configured_runtime().await;
        let old = start_stub(&runtime, &session, &agent);
        runtime
            .owned
            .provider_store
            .enable_workflow_tools(&old)
            .unwrap();
        // Reproduce persisted selection changing while an earlier provider run is still present.
        // Both adapters are deterministic test adapters; no provider process or credentials are used.
        runtime
            .owned
            .agent_store
            .set_agent_runtime_profile_with_account_profile(
                &agent,
                "managed-dev-stub",
                Some("next-model".into()),
                Some("low".into()),
                Some("next-account".into()),
                crate::provider::ProviderResumeState::default(),
            )
            .unwrap();
        let (next, _) = runtime
            .owned
            .workflow_ensure_provider_run(&session, &agent, false, false, false, fresh, None)
            .unwrap();
        let run = runtime.owned.provider_store.get_run(&next).unwrap();
        assert_eq!(
            run.adapter_key(),
            "managed-dev-stub",
            "adapter must follow the current provider, fresh={fresh}"
        );
        assert_eq!(run.provider(), "managed-dev-stub");
        assert_eq!(run.model(), "next-model");
        assert_eq!(run.account_profile(), "next-account");
        assert_eq!(
            runtime.owned.provider_store.get_run(&old).unwrap().state(),
            ProviderRunState::Ended
        );
    }
}
