//! `agent spawn` and a profile update refuse a model the account's catalog does not list.
use super::*;
use crate::provider::{OpenCodeProviderCatalog, OpenCodeProviderInfo, OpenCodeProviderModel};
use std::collections::BTreeMap;

fn claude_catalog() -> OpenCodeProviderCatalog {
    let model = |id: &str| OpenCodeProviderModel {
        id: id.to_string(),
        name: id.to_string(),
        status: "active".to_string(),
        limit: None,
        variants: BTreeMap::from([("low".to_string(), serde_json::json!({}))]),
    };
    OpenCodeProviderCatalog {
        all: vec![OpenCodeProviderInfo {
            id: "claude-p".to_string(),
            name: "claude-p".to_string(),
            remote_machine_aliases: Vec::new(),
            models: ["claude-haiku-5-5", "claude-sonnet-5-5"]
                .map(|id| (id.to_string(), model(id)))
                .into(),
        }],
        default: BTreeMap::from([("claude-p".to_string(), "claude-sonnet-5-5".to_string())]),
        connected: vec!["claude-p".to_string()],
    }
}

/// A Room with a claude account whose catalog the kernel already holds.
fn room_with_claude_account(
    harness: &LocalRouterTestHarness,
    worktree: &crate::test_support::TestWorktree,
) -> (String, String) {
    let LocalDaemonResponse::SessionCreated { session, .. } = harness
        .dispatch(LocalDaemonRequest::CreateSession(
            worktree.session_request(),
        ))
        .expect("session create should succeed")
    else {
        panic!("unexpected local response");
    };
    let owner = crate::session::DEFAULT_LOCAL_USER_ID;
    let profile = harness.with_app(|app| {
        let profile = app
            .provider_account_profile_registry()
            .create_managed(owner, "claude", "Work")
            .expect("account should be created");
        let request = crate::local::GetProviderCatalogRequest {
            provider: Some("claude-p".to_string()),
            account_profiles: BTreeMap::from([("claude".to_string(), profile.profile_id.clone())]),
            ..Default::default()
        };
        app.provider_catalog_projection_store().update_scoped(
            &crate::runtime::provider_catalog_control::provider_catalog_cache_key(owner, &request),
            claude_catalog(),
        );
        profile
    });
    (session.id().to_string(), profile.profile_id)
}

fn spawn_request(session_id: &str, profile_id: &str, model: &str) -> LocalDaemonRequest {
    LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
        max_context_tokens: None,
        no_subscribe: false,
        account_profile: Some(profile_id.to_string()),
        session_id: session_id.to_string(),
        alias: Some("worker".to_string()),
        provider: Some("claude-p".to_string()),
        model: Some(model.to_string()),
        effort: None,
        execution_mode: None,
        permission_level: None,
        worktree_id: None,
        kernel_ref: None,
        slice_ref: None,
        worktree_placement: None,
        metaagent: false,
    })
}

fn agent_count(harness: &LocalRouterTestHarness) -> usize {
    harness.with_app(|app| app.agents().list_agents().len())
}

#[test]
fn spawning_with_an_unknown_model_is_refused_and_creates_no_agent() {
    let worktree = crate::test_support::TestWorktree::new("spawn-model-unknown");
    let harness = LocalRouterTestHarness::new();
    let (session_id, profile_id) = room_with_claude_account(&harness, &worktree);
    let before = agent_count(&harness);

    let error = harness
        .dispatch(spawn_request(&session_id, &profile_id, "haiku-5-5"))
        .expect_err("an unknown model must be refused");

    assert!(
        error.to_string().contains(
            "unknown model \"haiku-5-5\" for provider claude-p; use one of: claude-haiku-5-5, claude-sonnet-5-5"
        ),
        "{error}"
    );
    assert_eq!(agent_count(&harness), before);
}

#[test]
fn spawning_with_a_listed_model_creates_the_agent() {
    let worktree = crate::test_support::TestWorktree::new("spawn-model-listed");
    let harness = LocalRouterTestHarness::new();
    let (session_id, profile_id) = room_with_claude_account(&harness, &worktree);

    let LocalDaemonResponse::AgentSpawned { agent, .. } = harness
        .dispatch(spawn_request(&session_id, &profile_id, "claude-haiku-5-5"))
        .expect("a listed model must spawn")
    else {
        panic!("unexpected local response");
    };

    assert_eq!(agent.model(), Some("claude-haiku-5-5"));
}

#[test]
fn changing_an_agent_to_an_unknown_model_is_refused() {
    let worktree = crate::test_support::TestWorktree::new("spawn-model-update");
    let harness = LocalRouterTestHarness::new();
    let (session_id, profile_id) = room_with_claude_account(&harness, &worktree);
    let LocalDaemonResponse::AgentSpawned { agent, .. } = harness
        .dispatch(spawn_request(&session_id, &profile_id, "claude-haiku-5-5"))
        .expect("a listed model must spawn")
    else {
        panic!("unexpected local response");
    };

    let error = harness
        .dispatch(LocalDaemonRequest::UpdateAgentProfile(
            UpdateAgentProfileRequest {
                session_id: session_id.clone(),
                agent_id: agent.id().to_string(),
                provider: None,
                account_profile: None,
                model: Some("haiku-5-5".to_string()),
                effort: None,
                clear_effort: false,
                reason: None,
            },
        ))
        .expect_err("an unknown model must be refused");

    assert!(
        error.to_string().contains("unknown model \"haiku-5-5\""),
        "{error}"
    );
}

fn provider_defaults(harness: &LocalRouterTestHarness) -> (Option<String>, Option<String>) {
    let LocalDaemonResponse::UserConfig { config, .. } = harness
        .dispatch(LocalDaemonRequest::GetUserConfig(
            crate::local::GetUserConfigRequest,
        ))
        .expect("user config should be readable")
    else {
        panic!("unexpected local response");
    };
    (config.providers.default, config.providers.model)
}

#[test]
fn creating_a_room_with_an_unknown_model_is_refused_and_saves_no_defaults() {
    let worktree = crate::test_support::TestWorktree::new("room-model-unknown");
    let harness = LocalRouterTestHarness::new();
    let (_, profile_id) = room_with_claude_account(&harness, &worktree);
    let before = provider_defaults(&harness);

    let error = harness
        .dispatch(LocalDaemonRequest::CreateSession(
            worktree
                .session_request()
                .with_agent_defaults(crate::session::SessionAgentDefaults {
                    account_profile: Some(profile_id),
                    ..crate::session::SessionAgentDefaults::new("claude-p").with_model("haiku-5-5")
                }),
        ))
        .expect_err("an unknown model must be refused");

    assert!(
        error.to_string().contains("unknown model \"haiku-5-5\""),
        "{error}"
    );
    assert_eq!(provider_defaults(&harness), before);
    harness
        .dispatch(LocalDaemonRequest::CreateSession(
            worktree.session_request(),
        ))
        .expect("a Room without explicit defaults must still be created");
}
