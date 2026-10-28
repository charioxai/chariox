use super::*;
use crate::session::CreateSessionRequest;

/// An agent saved before default names existed: same record, no alias.
fn unnamed_agent(
    app: &DaemonApp,
    session_id: &str,
    created_at_ms: u64,
) -> crate::agent::AgentInstance {
    let mut agent = {
        let mut sessions = app.sessions_mut();
        app.agents()
            .create_agent(
                crate::agent::CreateAgentRequest::new(session_id, "codex"),
                &mut sessions,
            )
            .unwrap()
    };
    agent.set_alias(None);
    // Agent ids are random, so the creation time is what orders the names.
    let mut record = serde_json::to_value(&agent).unwrap();
    record["created_at_ms"] = created_at_ms.into();
    app.agents()
        .restore_agent(serde_json::from_value(record).unwrap())
}

fn alias_of(app: &DaemonApp, agent_id: &str) -> Option<String> {
    app.agents()
        .read()
        .get_agent(agent_id)
        .unwrap()
        .alias()
        .map(str::to_string)
}

#[test]
fn names_given_while_restoring_old_agents_survive_a_restart_after_a_rename() {
    let config = DaemonConfig::for_tests();
    let (first, second) = {
        let app = DaemonApp::bootstrap(config.clone()).unwrap();
        let session = app
            .sessions_mut()
            .create_session(CreateSessionRequest::new(
                "workspace-names",
                "worktree-names",
            ))
            .unwrap();
        let first = unnamed_agent(&app, session.id(), 10);
        let second = unnamed_agent(&app, session.id(), 20);
        app.save_durable_state_snapshot().unwrap();
        (first.id().to_string(), second.id().to_string())
    };

    // The person sees agent-1 and agent-2, then renames the first agent. The
    // rename is a durable event for that agent only.
    {
        let app = DaemonApp::bootstrap(config.clone()).unwrap();
        assert_eq!(alias_of(&app, &first).as_deref(), Some("agent-1"));
        assert_eq!(alias_of(&app, &second).as_deref(), Some("agent-2"));
        let renamed = app
            .agents()
            .alias_agent(&first, Some("lead".to_string()))
            .unwrap();
        app.durable_state
            .append_event(
                "agent.updated",
                Some(renamed.id().to_string()),
                serde_json::json!({ "agent": renamed, "capability_name": null }),
            )
            .unwrap();
    }

    let app = DaemonApp::bootstrap(config).unwrap();
    assert_eq!(alias_of(&app, &first).as_deref(), Some("lead"));
    assert_eq!(alias_of(&app, &second).as_deref(), Some("agent-2"));
}
