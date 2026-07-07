use super::{calculate_agent_layout, AgentInstance, AgentSubstituteProfile, GridPosition};

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

#[test]
fn substitute_deactivation_restores_primary_without_default_model() {
    let mut agent = AgentInstance::new(
        "agent-1",
        "agent-1",
        "session-1",
        None,
        "opencode",
        None,
        None,
        None,
        GridPosition::new(0, 0, 1, 1),
    );
    agent.set_primary_profile("opencode", None, None);
    agent.add_substitute(AgentSubstituteProfile::new(
        "codex",
        "gpt-5.4",
        Some("medium".to_string()),
    ));

    let activated = agent.activate_substitute(0, "manual");
    assert_eq!(
        activated.as_ref().map(|profile| profile.provider.as_str()),
        Some("codex")
    );
    assert_eq!(agent.provider(), "codex");
    assert_eq!(agent.model(), Some("gpt-5.4"));
    assert_eq!(agent.effort(), Some("medium"));

    agent.deactivate_substitute();
    assert_eq!(agent.provider(), "opencode");
    assert_eq!(agent.model(), None);
    assert_eq!(agent.effort(), None);
}
