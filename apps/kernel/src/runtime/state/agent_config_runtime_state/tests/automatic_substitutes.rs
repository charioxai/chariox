use super::*;
use crate::account_profile::{
    ProviderAccountAuthState, ProviderAccountUsageAvailability, ProviderAccountUsageMeter,
    ProviderAccountUsageMeterKind, ProviderAccountUsageMeterScope, ProviderAccountUsageMeterState,
    ProviderAccountUsageSnapshot,
};

#[tokio::test]
async fn automatic_substitution_settles_failure_before_relaunching_queued_work() {
    let (runtime, session_id, agent_id, profile_id) =
        runtime_with_substitutes(&["opencode/deepseek-v4-pro"], true).await;
    runtime
        .owned
        .agent_store
        .set_agent_runtime_profile_with_account_profile(
            &agent_id,
            "opencode",
            Some("opencode-go/deepseek-v4-pro".to_string()),
            Some("high".to_string()),
            Some(profile_id.clone()),
            crate::provider::ProviderResumeState::default(),
        )
        .unwrap();
    let request = crate::provider::LaunchProviderRequest::new(
        &session_id,
        "opencode",
        "opencode",
        &profile_id,
        "opencode-go/deepseek-v4-pro",
    )
    .with_agent_id(&agent_id);
    let mut run = crate::provider::RuntimeProviderRun::new(
        "failed-go-run",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "test-go".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("test-go-runtime".to_string()),
        },
    );
    run.mark_running();
    let queued_before = runtime
        .with_app_side_effect(|app| {
            app.providers_mut().insert_run_for_test(run.clone());
            app.sessions_mut()
                .set_active_provider_run(&session_id, Some(run.id().to_string()))?;
            let attachment = crate::app::KernelSessionService::new(app).attach(
                crate::attachment::AttachRequest::new(
                    &session_id,
                    "substitute-test",
                    crate::attachment::ClientCapabilityLevel::FullTerminal,
                ),
            )?;
            for prompt_id in ["failed-prompt", "queued-prompt"] {
                let prompt = crate::session::PromptQueueItem::new(
                    prompt_id,
                    attachment.id(),
                    &agent_id,
                    "test prompt",
                    crate::session::PromptStatus::Queued,
                );
                app.prompt_owner_submit_prepared_prompt(&session_id, prompt, false)?;
            }
            Ok::<_, DaemonError>(
                app.prompt_owner_peek_next_queued_prompt(&session_id, &agent_id)?
                    .expect("second prompt has a kernel-assigned queue identity"),
            )
        })
        .await
        .unwrap();
    let message = crate::provider::classify_provider_substitutable_failure_text(
        "opencode",
        "insufficient balance",
    )
    .unwrap();
    runtime
        .fail_owned_provider_prompt(&session_id, run.id(), &message, true)
        .await
        .expect("pending work must not relaunch the exhausted account before failover");
    let agent = runtime.owned.agent_store.get_agent(&agent_id).unwrap();
    assert_eq!(agent.active_substitute_index(), Some(0));
    assert_eq!(agent.model(), Some("opencode/deepseek-v4-pro"));
    let session = runtime
        .owned
        .session_store
        .get_session(&session_id)
        .unwrap();
    let queued = runtime
        .owned
        .prompt_state_owner
        .peek_next_queued_prompt(&session, &agent_id)
        .unwrap();
    assert_eq!(
        serde_json::to_value(queued).unwrap(),
        serde_json::to_value(queued_before).unwrap(),
        "existing work remains queued without replaying failed work"
    );
    let (active, pending) = runtime
        .owned
        .prompt_state_owner
        .state_parts(&session, &agent_id);
    assert!(active.is_none());
    assert_eq!(pending.len(), 1);
}

async fn runtime_with_substitutes(
    models: &[&str],
    reset_in_future: bool,
) -> (KernelRuntimeState, String, String, String) {
    let (app, runtime, session_id, agent_id) = agent_config_runtime().await;
    let registry = app.lock().await.provider_account_profile_registry();
    let profile = registry
        .create_managed(
            crate::session::DEFAULT_LOCAL_USER_ID,
            "opencode",
            "Go and Zen",
        )
        .expect("isolated account");
    registry
        .update_observation(
            crate::session::DEFAULT_LOCAL_USER_ID,
            "opencode",
            &profile.profile_id,
            ProviderAccountAuthState::Authenticated,
            None,
            None,
            None,
            None,
        )
        .expect("authenticated fixture metadata");
    let now_ms = crate::session::unix_epoch_ms();
    registry
        .update_usage(
            crate::session::DEFAULT_LOCAL_USER_ID,
            "opencode",
            &profile.profile_id,
            ProviderAccountUsageSnapshot {
                profile_id: profile.profile_id.clone(),
                provider: "opencode".to_string(),
                availability: ProviderAccountUsageAvailability::Available,
                meters: vec![ProviderAccountUsageMeter {
                    meter_id: "go/monthly".to_string(),
                    label: "Go monthly".to_string(),
                    service_id: Some("opencode-go".to_string()),
                    kind: ProviderAccountUsageMeterKind::RollingLimit,
                    scope: ProviderAccountUsageMeterScope::Plan,
                    used_percent: Some(100.0),
                    used: None,
                    remaining: None,
                    total: None,
                    unit: None,
                    window_duration_minutes: None,
                    resets_at_ms: Some(if reset_in_future { now_ms + 60_000 } else { 0 }),
                    state: ProviderAccountUsageMeterState::Exhausted,
                    source: "test".to_string(),
                    observed_at_ms: now_ms,
                }],
                observed_at_ms: Some(now_ms),
                source: "test".to_string(),
                management_url: None,
            },
        )
        .expect("known exhausted Go, unreported Zen");
    for &model in models {
        runtime
            .owned
            .agent_store
            .add_agent_substitute(
                &agent_id,
                crate::agent::AgentSubstituteProfile::new(
                    "opencode",
                    model,
                    Some("high".to_string()),
                )
                .with_account_profile(Some(profile.profile_id.clone())),
            )
            .expect("configured substitute");
    }
    (runtime, session_id, agent_id, profile.profile_id)
}

#[tokio::test]
async fn automatic_substitution_skips_exhausted_go_but_preserves_zen_on_same_account() {
    let (runtime, session_id, agent_id, profile_id) = runtime_with_substitutes(
        &["opencode-go/deepseek-v4-pro", "opencode/deepseek-v4-pro"],
        true,
    )
    .await;
    let starter = runtime.owned.agent_store.get_agent(&agent_id).unwrap();
    let reason = crate::provider::classify_provider_substitutable_failure_text(
        "claude", "Provider prompt dispatch failed: Provider reported a substitutable resource limit: You've hit your session limit",
    ).expect("real Claude failure projection");
    assert!(runtime
        .activate_next_agent_substitute_after_failure(&session_id, &agent_id, &reason)
        .await
        .expect("automatic selection"));
    let selected = runtime.owned.agent_store.get_agent(&agent_id).unwrap();
    assert_eq!(
        selected.active_substitute_index(),
        Some(1),
        "exhausted Go must not strand the chain before Zen"
    );
    assert_eq!(selected.model(), Some("opencode/deepseek-v4-pro"));
    assert_eq!(selected.provider_account_profile(), profile_id);
    assert_eq!(selected.primary_provider(), starter.provider());
    assert_eq!(selected.primary_model(), starter.model());
    assert_eq!(
        selected.substitutes().len(),
        2,
        "configured order remains intact"
    );
    // This current-thread test does not yield to the spawned launch task. It
    // tests the real selection path without launching a provider or spending credit.
}

#[tokio::test]
async fn automatic_substitution_exhausted_chain_leaves_starter_unchanged() {
    let (runtime, session_id, agent_id, _) = runtime_with_substitutes(
        &[
            "opencode-go/deepseek-v4-pro",
            "opencode-go/deepseek-v4-flash",
        ],
        true,
    )
    .await;
    let before = runtime.owned.agent_store.get_agent(&agent_id).unwrap();
    assert!(!runtime
        .activate_next_agent_substitute_after_failure(&session_id, &agent_id, "usage limit")
        .await
        .unwrap());
    let after = runtime.owned.agent_store.get_agent(&agent_id).unwrap();
    assert_eq!(
        serde_json::to_value(after).unwrap(),
        serde_json::to_value(before).unwrap()
    );
    assert!(runtime.owned.provider_store.list_runs().is_empty());
}

#[tokio::test]
async fn automatic_substitution_does_not_skip_a_passed_reset() {
    let (runtime, session_id, agent_id, _) = runtime_with_substitutes(
        &["opencode-go/deepseek-v4-pro", "opencode/deepseek-v4-pro"],
        false,
    )
    .await;
    assert!(runtime
        .activate_next_agent_substitute_after_failure(&session_id, &agent_id, "usage limit")
        .await
        .unwrap());
    assert_eq!(
        runtime
            .owned
            .agent_store
            .get_agent(&agent_id)
            .unwrap()
            .active_substitute_index(),
        Some(0)
    );
}

#[tokio::test]
async fn automatic_substitution_skips_multiple_exhausted_entries_after_active_index() {
    let (runtime, session_id, agent_id, _) = runtime_with_substitutes(
        &[
            "opencode-go/deepseek-v4-pro",
            "opencode-go/deepseek-v4-flash",
            "opencode/deepseek-v4-pro",
        ],
        true,
    )
    .await;
    runtime
        .owned
        .agent_store
        .activate_agent_substitute(&agent_id, 0, "previous selection".to_string())
        .unwrap();
    assert!(runtime
        .activate_next_agent_substitute_after_failure(&session_id, &agent_id, "usage limit")
        .await
        .unwrap());
    assert_eq!(
        runtime
            .owned
            .agent_store
            .get_agent(&agent_id)
            .unwrap()
            .active_substitute_index(),
        Some(2)
    );
}
