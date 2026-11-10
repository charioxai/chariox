//! An agent always stores a concrete model and variant, chosen from its account's catalog.
use super::*;
use crate::provider::{OpenCodeProviderCatalog, OpenCodeProviderInfo, OpenCodeProviderModel};
use std::collections::BTreeMap;

fn model(id: &str, variants: &[&str]) -> OpenCodeProviderModel {
    OpenCodeProviderModel {
        id: id.to_string(),
        name: id.to_string(),
        status: "active".to_string(),
        limit: None,
        variants: variants
            .iter()
            .map(|variant| (variant.to_string(), serde_json::json!({})))
            .collect(),
    }
}

fn provider(id: &str, models: &[(&str, &[&str])]) -> OpenCodeProviderInfo {
    OpenCodeProviderInfo {
        id: id.to_string(),
        name: id.to_string(),
        remote_machine_aliases: Vec::new(),
        models: models
            .iter()
            .map(|(model_id, variants)| (model_id.to_string(), model(model_id, variants)))
            .collect(),
    }
}

/// What a switch to the provider must store with `catalog_for`'s default, and the variant.
pub(super) fn default_selection(provider: &str) -> (&'static str, Option<&'static str>) {
    match provider {
        "codex" => ("gpt-6.1-sol", Some("high")),
        "claude-p" | "claude-headless" => ("claude-sonnet-5-5", Some("high")),
        _ => ("opencode-go/kimi", None),
    }
}

fn catalog_for(provider_name: &str) -> OpenCodeProviderCatalog {
    match provider_name {
        "codex" => OpenCodeProviderCatalog {
            all: vec![provider(
                "codex",
                &[
                    ("gpt-6-luna", &["low", "medium"]),
                    ("gpt-6.1-sol", &["low", "medium", "high"]),
                ],
            )],
            default: BTreeMap::from([("codex".to_string(), "gpt-6.1-sol".to_string())]),
            connected: vec!["codex".to_string()],
        },
        "claude-p" | "claude-headless" => OpenCodeProviderCatalog {
            all: vec![provider(
                provider_name,
                &[
                    ("claude-haiku-5-5", &["low"]),
                    ("claude-sonnet-5-5", &["low", "high"]),
                ],
            )],
            default: BTreeMap::from([(provider_name.to_string(), "claude-sonnet-5-5".to_string())]),
            connected: vec![provider_name.to_string()],
        },
        _ => OpenCodeProviderCatalog {
            all: vec![provider("opencode-go", &[("kimi", &[]), ("glm", &[])])],
            default: BTreeMap::from([("opencode-go".to_string(), "kimi".to_string())]),
            connected: vec!["opencode-go".to_string()],
        },
    }
}

/// Puts the catalog the kernel would read for the account into the projection, as a live read does.
pub(super) async fn seed_account_catalog(
    app: &Arc<Mutex<DaemonApp>>,
    owner: &str,
    provider_name: &str,
    profile_id: &str,
) {
    let family = crate::provider::canonical_provider_family(provider_name).unwrap();
    let request = crate::local::GetProviderCatalogRequest {
        provider: Some(provider_name.to_string()),
        account_profiles: BTreeMap::from([(family.to_string(), profile_id.to_string())]),
        ..Default::default()
    };
    let key = crate::runtime::provider_catalog_control::provider_catalog_cache_key(owner, &request);
    app.lock()
        .await
        .provider_catalog_projection_store()
        .update_scoped(&key, catalog_for(provider_name));
}

#[tokio::test]
async fn an_account_switch_keeps_the_model_the_new_account_offers() {
    let (app, runtime, session, agent) = agent_config_runtime().await;
    let registry = app.lock().await.provider_account_profile_registry();
    let owner = crate::session::DEFAULT_LOCAL_USER_ID;
    let first = registry.create_managed(owner, "codex", "First").unwrap();
    let second = registry.create_managed(owner, "codex", "Second").unwrap();
    runtime
        .update_agent_profile(
            &session,
            &agent,
            owner,
            Some("codex".into()),
            Some(first.profile_id.clone()),
            Some("gpt-6-luna".into()),
            Some(Some("medium".into())),
        )
        .await
        .unwrap();
    seed_account_catalog(&app, owner, "codex", &second.profile_id).await;
    let switched = runtime
        .update_agent_profile(
            &session,
            &agent,
            owner,
            Some("codex".into()),
            Some(second.profile_id.clone()),
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(switched.provider_account_profile(), second.profile_id);
    assert_eq!(switched.model(), Some("gpt-6-luna"));
    assert_eq!(switched.effort(), Some("medium"));
}

#[tokio::test]
async fn an_account_switch_replaces_a_model_the_new_account_lacks() {
    let (app, runtime, session, agent) = agent_config_runtime().await;
    let registry = app.lock().await.provider_account_profile_registry();
    let owner = crate::session::DEFAULT_LOCAL_USER_ID;
    let first = registry.create_managed(owner, "codex", "First").unwrap();
    let second = registry.create_managed(owner, "codex", "Second").unwrap();
    runtime
        .update_agent_profile(
            &session,
            &agent,
            owner,
            Some("codex".into()),
            Some(first.profile_id.clone()),
            Some("retired-model".into()),
            Some(Some("low".into())),
        )
        .await
        .unwrap();
    seed_account_catalog(&app, owner, "codex", &second.profile_id).await;
    let switched = runtime
        .update_agent_profile(
            &session,
            &agent,
            owner,
            Some("codex".into()),
            Some(second.profile_id.clone()),
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(switched.model(), Some("gpt-6.1-sol"));
    assert_eq!(switched.effort(), Some("low"));
}

#[tokio::test]
async fn a_new_agent_and_a_new_room_get_the_account_default() {
    let (app, runtime, _session, _agent) = agent_config_runtime().await;
    let registry = app.lock().await.provider_account_profile_registry();
    let owner = crate::session::DEFAULT_LOCAL_USER_ID;
    let profile = registry.create_managed(owner, "claude", "Work").unwrap();
    seed_account_catalog(&app, owner, "claude-p", &profile.profile_id).await;
    let (model, effort) = runtime
        .resolve_new_agent_selection(owner, "claude-p", Some(&profile.profile_id), None, None)
        .await
        .unwrap();
    assert_eq!(model.as_deref(), Some("claude-sonnet-5-5"));
    assert_eq!(effort.as_deref(), Some("high"));

    registry
        .set_default(owner, "claude", &profile.profile_id)
        .unwrap();
    seed_account_catalog(&app, owner, "claude-p", &profile.profile_id).await;
    let defaults = runtime
        .resolve_new_session_defaults(
            owner,
            Some(crate::session::SessionAgentDefaults {
                provider: "claude-p".into(),
                ..Default::default()
            }),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(defaults.model.as_deref(), Some("claude-sonnet-5-5"));
    assert_eq!(defaults.effort.as_deref(), Some("high"));
}

async fn stored_codex_agent_without_model(
) -> (Arc<Mutex<DaemonApp>>, KernelRuntimeState, String, String) {
    let request = crate::session::CreateSessionRequest::new("workspace-1", "worktree-1")
        .with_agent_defaults(crate::session::SessionAgentDefaults {
            provider: "codex".into(),
            ..Default::default()
        });
    agent_config_runtime_in_worktree(
        crate::config::DaemonConfig::for_tests(),
        crate::session::DEFAULT_LOCAL_USER_ID,
        request,
    )
    .await
}

#[tokio::test]
async fn start_up_gives_a_stored_agent_without_model_its_account_default() {
    let (app, runtime, _session, agent) = stored_codex_agent_without_model().await;
    let owner = crate::session::DEFAULT_LOCAL_USER_ID;
    let stored = runtime
        .list_agents()
        .into_iter()
        .find(|a| a.id() == agent)
        .unwrap();
    assert!(stored.model().is_none() && stored.effort().is_none());
    seed_account_catalog(&app, owner, "codex", stored.provider_account_profile()).await;

    runtime.resolve_stored_agent_models().await;

    let resolved = runtime
        .list_agents()
        .into_iter()
        .find(|a| a.id() == agent)
        .unwrap();
    assert_eq!(resolved.model(), Some("gpt-6.1-sol"));
    assert_eq!(resolved.effort(), Some("high"));
    assert!(runtime.agents_missing_model_choice().is_empty());
}

#[tokio::test]
async fn start_up_leaves_an_agent_that_already_has_model_and_variant_as_it_is() {
    let (_app, runtime, session, agent) = stored_codex_agent_without_model().await;
    let owner = crate::session::DEFAULT_LOCAL_USER_ID;
    runtime
        .update_agent_profile(
            &session,
            &agent,
            owner,
            None,
            None,
            Some("retired-model".into()),
            Some(Some("low".into())),
        )
        .await
        .unwrap();
    assert!(runtime.agents_missing_model_choice().is_empty());

    runtime.resolve_stored_agent_models().await;

    let kept = runtime
        .list_agents()
        .into_iter()
        .find(|a| a.id() == agent)
        .unwrap();
    assert_eq!(kept.model(), Some("retired-model"));
    assert_eq!(kept.effort(), Some("low"));
}

// MP-08 / MP-10: a model whose catalog entry has no variants is complete, so a restart neither
// rewrites the agent nor retires the workflow copies that were made from it.
#[tokio::test]
async fn start_up_keeps_a_variantless_model_and_the_workflow_copies_made_from_it() {
    let request = crate::session::CreateSessionRequest::new("workspace-1", "worktree-1")
        .with_agent_defaults(crate::session::SessionAgentDefaults {
            provider: "claude-p".into(),
            ..Default::default()
        });
    let (app, runtime, session, agent) = agent_config_runtime_in_worktree(
        crate::config::DaemonConfig::for_tests(),
        crate::session::DEFAULT_LOCAL_USER_ID,
        request,
    )
    .await;
    super::seeded_catalog::seed_claude_aliases(&app, "claude-p").await;
    runtime
        .update_agent_profile(
            &session,
            &agent,
            crate::session::DEFAULT_LOCAL_USER_ID,
            None,
            None,
            Some("haiku".into()),
            None,
        )
        .await
        .unwrap();
    let (workflow, revision) = {
        let mut store = runtime.owned.session_store.write();
        let workflow = store
            .create_workflow(&session, Some("review".into()))
            .unwrap();
        let node = store
            .add_workflow_node(&session, workflow.id(), &agent)
            .unwrap();
        let endpoint = store
            .create_workflow_endpoint(&session, workflow.id(), node.id(), Some("entry".into()))
            .unwrap();
        let workflow = store.resolve_workflow_ref(&session, workflow.id()).unwrap();
        let node_agents = workflow
            .nodes()
            .iter()
            .map(|node| (node.id().to_string(), node.agent_id().to_string()))
            .collect();
        store
            .register_workflow_runtime_instance(
                &session,
                crate::session::WorkflowEndpointRuntimeInstance::new(
                    "instance-copy",
                    workflow.id(),
                    endpoint.id(),
                    workflow.revision(),
                    2,
                    false,
                    node_agents,
                    "worktree-2",
                ),
            )
            .unwrap();
        (workflow.id().to_string(), workflow.revision())
    };
    let kept = runtime
        .list_agents()
        .into_iter()
        .find(|a| a.id() == agent)
        .unwrap();
    assert!(kept.model() == Some("haiku") && kept.effort().is_none());

    for _ in 0..2 {
        runtime.resolve_stored_agent_models().await;
    }

    let snapshot = runtime.owned.session_store.get_session(&session).unwrap();
    let after = snapshot.workflow(&workflow).unwrap();
    assert!(
        after.revision() == revision,
        "a restart bumped the workflow from revision {revision} to {}",
        after.revision()
    );
    let copy = snapshot.workflow_runtime_instance("instance-copy");
    assert!(
        copy.is_some_and(|copy| copy.workflow_revision() == revision),
        "the workflow copy was retired by a restart"
    );
}
