use super::{
    calculate_agent_layout, AgentInstance, AgentState, AgentSubstituteProfile, GridPosition,
};

#[test]
fn calculate_agent_layout_expands_past_six_agents() {
    assert_eq!(
        calculate_agent_layout(7),
        vec![
            GridPosition::new(0, 0, 1, 1),
            GridPosition::new(0, 1, 1, 1),
            GridPosition::new(0, 2, 1, 1),
            GridPosition::new(0, 3, 1, 1),
            GridPosition::new(1, 0, 1, 1),
            GridPosition::new(1, 1, 1, 1),
            GridPosition::new(1, 2, 1, 1),
        ]
    );
}

fn agent(provider: &str, model: &str) -> AgentInstance {
    AgentInstance::new(
        "agent-1",
        "agent-1",
        "session-1",
        None,
        provider,
        Some(model.to_string()),
        None,
        None,
        GridPosition::new(0, 0, 1, 1),
    )
}

#[test]
fn agent_persisted_on_a_substitute_loads_on_its_primary_profile() {
    let legacy = serde_json::json!({
        "id": "agent-1",
        "agent_ref": "agent-1",
        "session_id": "session-1",
        "alias": null,
        "provider": "claude",
        "model": "claude-opus-5-5",
        "effort": "max",
        "account_profile": "claude-personal",
        "primary_provider": "codex",
        "primary_model": "gpt-6.1-sol",
        "primary_effort": "high",
        "primary_account_profile": "codex-work",
        "substitutes": [
            {"provider": "claude", "model": "claude-opus-5-5", "variant": "max",
             "account_profile": "claude-personal"}
        ],
        "active_substitute_index": 0,
        "last_substitution": {"substitute_index": 0, "reason": "usage limit", "activated_at_ms": 1},
        "state": "Idle",
        "is_processing": false,
        "position": {"row": 0, "col": 0, "row_span": 1, "col_span": 1},
        "created_at_ms": 1,
        "last_activity_at_ms": 1
    });

    let restored: AgentInstance = serde_json::from_value(legacy).expect("legacy agent loads");

    assert_eq!(restored.provider(), "codex");
    assert_eq!(restored.model(), Some("gpt-6.1-sol"));
    assert_eq!(restored.effort(), Some("high"));
    assert_eq!(restored.account_profile(), Some("codex-work"));
    assert_eq!(restored.substitutes().len(), 1, "the fallback list is kept");
    let reserialized = serde_json::to_value(&restored).expect("agent serializes");
    for retired in [
        "primary_provider",
        "primary_model",
        "primary_effort",
        "primary_account_profile",
        "active_substitute_index",
        "last_substitution",
    ] {
        assert!(reserialized.get(retired).is_none(), "{retired} is retired");
    }
}

#[test]
fn legacy_primary_snapshot_without_an_active_substitute_does_not_revert_later_edits() {
    // A returned-to-primary agent kept its stale `primary_*` snapshot; the
    // live profile is authoritative and must not be rolled back on load.
    let mut current = serde_json::to_value(agent("codex", "gpt-6.1-sol")).unwrap();
    current["primary_provider"] = serde_json::json!("opencode");
    current["primary_model"] = serde_json::json!("deepseek-v4-pro");
    current["last_substitution"] = serde_json::Value::Null;

    let restored: AgentInstance = serde_json::from_value(current).expect("agent loads");

    assert_eq!(restored.provider(), "codex");
    assert_eq!(restored.model(), Some("gpt-6.1-sol"));
}

#[test]
fn legacy_substitute_on_the_default_account_restores_the_default_primary_account() {
    let mut legacy = serde_json::to_value(agent("claude", "claude-opus-5-5")).unwrap();
    legacy["account_profile"] = serde_json::json!("claude-personal");
    legacy["primary_provider"] = serde_json::json!("codex");
    legacy["primary_model"] = serde_json::json!("gpt-6.1-sol");
    legacy["active_substitute_index"] = serde_json::json!(0);

    let restored: AgentInstance = serde_json::from_value(legacy).expect("agent loads");

    assert_eq!(restored.provider(), "codex");
    assert_eq!(restored.effort(), None);
    assert_eq!(restored.account_profile(), None);
    assert_eq!(restored.provider_account_profile(), "default");
}

#[test]
fn substitute_list_edits_keep_order_and_never_change_the_configured_profile() {
    let mut agent = agent("claude", "claude-opus-4-8");
    for (provider, model) in [
        ("opencode", "opencode-go/deepseek-v4-pro"),
        ("opencode", "deepseek-v4-pro"),
        ("codex", "gpt-5.6-sol"),
    ] {
        agent.add_substitute(AgentSubstituteProfile::new(provider, model, None));
    }
    let models = |agent: &AgentInstance| {
        agent
            .substitutes()
            .iter()
            .map(|profile| profile.model.clone())
            .collect::<Vec<_>>()
    };

    assert!(agent.move_substitute(1, 0));
    assert_eq!(
        models(&agent),
        [
            "deepseek-v4-pro",
            "opencode-go/deepseek-v4-pro",
            "gpt-5.6-sol"
        ]
    );
    assert!(agent.remove_substitute(0).is_some());
    assert_eq!(
        models(&agent),
        ["opencode-go/deepseek-v4-pro", "gpt-5.6-sol"]
    );
    assert!(!agent.move_substitute(2, 0));
    assert!(agent.remove_substitute(2).is_none());
    agent.clear_substitutes();
    assert!(agent.substitutes().is_empty());
    assert_eq!(agent.provider(), "claude");
    assert_eq!(agent.model(), Some("claude-opus-4-8"));
}

#[test]
fn substitute_profile_preserves_account_profile_binding_and_default_semantics() {
    let default_profile = AgentSubstituteProfile::new("codex", "gpt-5.4", None);
    assert_eq!(default_profile.account_profile, None);

    let bound = AgentSubstituteProfile::new("codex", "gpt-5.4", None)
        .with_account_profile(Some("  work  ".to_string()));
    assert_eq!(bound.account_profile.as_deref(), Some("work"));

    let cleared = AgentSubstituteProfile::new("codex", "gpt-5.4", None)
        .with_account_profile(Some("   ".to_string()));
    assert_eq!(cleared.account_profile, None);

    // Persistence round-trip: the bound profile survives serialization and a
    // legacy profile without account_profile deserializes back to `None`.
    let serialized = serde_json::to_string(&bound).expect("substitute profile should serialize");
    let deserialized: AgentSubstituteProfile =
        serde_json::from_str(&serialized).expect("substitute profile should deserialize");
    assert_eq!(deserialized, bound);
    let legacy: AgentSubstituteProfile =
        serde_json::from_str(r#"{"provider":"codex","model":"gpt-5.4"}"#)
            .expect("legacy substitute profile should deserialize");
    assert_eq!(legacy.account_profile, None);
}

#[test]
fn workflow_runtime_materialization_preserves_config_without_live_state() {
    let mut source = AgentInstance::new(
        "agent-1",
        "source-ref",
        "session-1",
        Some("reviewer".to_string()),
        "opencode",
        Some("x-preview-f-free".to_string()),
        Some("high".to_string()),
        Some("/source".to_string()),
        GridPosition::new(0, 0, 1, 1),
    );
    source.set_account_profile(Some("zen".to_string()));
    source.set_execution_mode_override(Some(crate::provider::AgentExecutionMode::Build));
    source.set_permission_level_override(Some(crate::provider::AgentPermissionLevel::Yolo));
    source.grant_mcp("github");
    source.add_substitute(
        AgentSubstituteProfile::new("codex", "gpt-5.6-sol", Some("high".to_string()))
            .with_account_profile(Some("codex-work".to_string())),
    );
    source.set_provider_resume_state(
        crate::provider::ProviderResumeState::from_opencode_session_id("provider-session-secret"),
    );
    source.set_state(AgentState::Working);
    source.set_processing(true);

    let substitutes = source.substitutes().to_vec();
    let runtime = source.materialized_for_workflow_runtime(
        "agent-runtime",
        "runtime-ref",
        "session-1",
        "/isolated",
    );

    assert_eq!(runtime.provider(), "opencode");
    assert_eq!(runtime.model(), Some("x-preview-f-free"));
    assert_eq!(runtime.effort(), Some("high"));
    assert_eq!(runtime.account_profile(), Some("zen"));
    assert_eq!(runtime.substitutes(), substitutes);
    assert_eq!(
        runtime.execution_mode_override(),
        Some(crate::provider::AgentExecutionMode::Build)
    );
    assert_eq!(
        runtime.permission_level_override(),
        Some(crate::provider::AgentPermissionLevel::Yolo)
    );
    assert_eq!(runtime.mcp_grants(), vec!["github".to_string()]);
    assert_eq!(runtime.worktree_id(), Some("/isolated"));
    assert_eq!(runtime.state(), AgentState::Idle);
    assert!(!runtime.is_processing());
    assert!(runtime.provider_resume_state().is_empty());
    assert!(!runtime.visible_in_freeform());
}
