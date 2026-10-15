//! MP-08 / MP-10: provider controls share kernel-owned account selection.
use super::*;

#[tokio::test]
async fn provider_family_switch_uses_target_default_and_clears_implicit_model() {
    for source in ["codex", "claude-p", "opencode"] {
        for target in ["codex", "claude-p", "opencode"] {
            if source == target {
                continue;
            }
            let (app, runtime, session, agent) = agent_config_runtime().await;
            let registry = app.lock().await.provider_account_profile_registry();
            let owner = crate::session::DEFAULT_LOCAL_USER_ID;
            let source_profile = registry.create_managed(owner, source, "Source").unwrap();
            let target_profile = registry.create_managed(owner, target, "Target").unwrap();
            registry
                .set_default(owner, target, &target_profile.profile_id)
                .unwrap();
            runtime
                .update_agent_profile(
                    &session,
                    &agent,
                    owner,
                    Some(source.into()),
                    Some(source_profile.profile_id.clone()),
                    Some("source-model".into()),
                    Some(Some("high".into())),
                )
                .await
                .unwrap();
            let switched = runtime
                .update_agent_profile(
                    &session,
                    &agent,
                    owner,
                    Some(target.into()),
                    None,
                    None,
                    None,
                )
                .await
                .expect("provider-only control must resolve the target family");
            assert_eq!(switched.provider(), target);
            assert_eq!(
                switched.provider_account_profile(),
                target_profile.profile_id
            );
            assert_eq!(switched.model(), None);
            assert_eq!(switched.effort(), None);
        }
    }
}

#[tokio::test]
async fn same_family_switch_keeps_pinned_account_model_and_effort() {
    let (app, runtime, session, agent) = agent_config_runtime().await;
    let registry = app.lock().await.provider_account_profile_registry();
    let owner = crate::session::DEFAULT_LOCAL_USER_ID;
    let pinned = registry.create_managed(owner, "claude", "Pinned").unwrap();
    runtime
        .update_agent_profile(
            &session,
            &agent,
            owner,
            Some("claude-p".into()),
            Some(pinned.profile_id.clone()),
            Some("sonnet".into()),
            Some(Some("high".into())),
        )
        .await
        .unwrap();
    let other = registry.create_managed(owner, "claude", "Other").unwrap();
    registry
        .set_default(owner, "claude", &other.profile_id)
        .unwrap();
    let switched = runtime
        .update_agent_profile(
            &session,
            &agent,
            owner,
            Some("claude-headless".into()),
            None,
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(switched.provider_account_profile(), pinned.profile_id);
    assert_eq!(switched.model(), Some("sonnet"));
    assert_eq!(switched.effort(), Some("high"));
}

#[tokio::test]
async fn provider_switch_validates_explicit_account_and_preserves_explicit_selection() {
    let (app, runtime, session, agent) = agent_config_runtime().await;
    let registry = app.lock().await.provider_account_profile_registry();
    let owner = crate::session::DEFAULT_LOCAL_USER_ID;
    let source = registry.create_managed(owner, "codex", "Source").unwrap();
    let target = registry
        .create_managed(owner, "opencode", "Target")
        .unwrap();
    runtime
        .update_agent_profile(
            &session,
            &agent,
            owner,
            Some("codex".into()),
            Some(source.profile_id.clone()),
            Some("gpt-6.1-sol".into()),
            None,
        )
        .await
        .unwrap();
    runtime
        .update_agent_profile(
            &session,
            &agent,
            owner,
            Some("opencode".into()),
            Some(source.profile_id.clone()),
            None,
            None,
        )
        .await
        .expect_err("an explicit wrong-family account must still be rejected");
    let unchanged = runtime.owned.agent_store.get_agent(&agent).unwrap();
    assert_eq!(unchanged.provider(), "codex");
    assert_eq!(unchanged.provider_account_profile(), source.profile_id);
    let switched = runtime
        .update_agent_profile(
            &session,
            &agent,
            owner,
            Some("opencode".into()),
            Some(target.profile_id.clone()),
            Some("openai/gpt-6.1-sol".into()),
            Some(Some("low".into())),
        )
        .await
        .unwrap();
    assert_eq!(switched.provider_account_profile(), target.profile_id);
    assert_eq!(switched.model(), Some("openai/gpt-6.1-sol"));
    assert_eq!(switched.effort(), Some("low"));
}
