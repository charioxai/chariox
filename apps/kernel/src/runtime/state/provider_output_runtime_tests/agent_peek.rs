//! MP-08 / MP-09 / MP-10 / MP-11: transcript reads use the same agent authorization and discovery as other reads.
use super::*;

const TOKEN: &str = "agent-peek-token";

async fn runtime_with_agent(root: &crate::test_support::TestWorktree, meta: bool) -> KernelRuntimeState {
    let mut app =
        crate::test_support::bootstrap_authenticated_app(crate::config::DaemonConfig::for_tests())
            .unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            root.path().display().to_string(),
            root.path().display().to_string(),
        ))
        .unwrap();
    if meta {
        app.agents_mut().activate_agent_meta_mode(agent.id(), None).unwrap();
    }
    let launch = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "dev-stub",
        "dev-stub",
        "default",
        "test",
    )
    .with_agent_id(agent.id());
    let mut run = crate::provider::RuntimeProviderRun::new(
        "agent-peek-run",
        &launch,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "agent peek".into(),
            pty_target: None,
            pty_program: None,
            pty_args: vec![],
            pty_env: Default::default(),
            pty_env_remove: vec![],
            working_directory: Some(root.path().to_owned()),
            structured_endpoint: None,
        },
    );
    run.mark_running();
    run.set_runtime_mcp_auth_token(Some(TOKEN.into()));
    app.providers_mut().insert_run_for_test(run);
    owned_runtime_state(&Arc::new(Mutex::new(app))).await
}

#[tokio::test]
async fn agent_peek_is_offered_and_dispatched_for_ordinary_and_meta_agents() {
    crate::test_support::isolated_env_test!();
    for meta in [false, true] {
        let root = crate::test_support::TestWorktree::new("agent-peek-dispatch");
        let runtime = runtime_with_agent(&root, meta).await;
        let run = runtime.owned.provider_store.get_run("agent-peek-run").unwrap();
        let id = run.agent_instance_id().unwrap();
        let names = runtime.runtime_tool_specs_for_auth_token(TOKEN)
            .into_iter().map(|s| s.name).collect::<Vec<_>>();
        for tool in ["chariox.peek_agent", "chariox.read_agent_entry"] {
            assert!(names.iter().any(|n| n == tool), "meta={meta}: {tool} missing");
        }
        let result = runtime.dispatch_authenticated_runtime_tool_call(
            TOKEN, "mcp__chariox__chariox_peek_agent", serde_json::json!({"agent": id})
        ).await.unwrap();
        assert!(result.ok, "meta={meta}: {}", result.payload);
        assert_eq!(result.payload["agent_id"], id);
        assert!(result.payload["entries"].is_array());
        let refused = runtime.dispatch_authenticated_runtime_tool_call(
            TOKEN, "chariox.peek_agent", serde_json::json!({"agent": "outside-this-room"})
        ).await.unwrap();
        assert!(!refused.ok, "an agent from outside the room must be refused");
    }
}

#[tokio::test]
async fn agent_peek_redacts_values_before_shortening_titles_and_slicing_pages() {
    crate::test_support::isolated_env_test!();
    let root = crate::test_support::TestWorktree::new("agent-peek-protection");
    let runtime = runtime_with_agent(&root, false).await;
    let run = runtime.owned.provider_store.get_run("agent-peek-run").unwrap();
    let room = run.session_id();
    let agent = run.agent_instance_id().unwrap();
    let secret = "abcdefgh-secret-value";
    runtime.owned.room_secret_observations.register(room, secret).unwrap();
    let context = crate::history::HistoryEventTurnContext {
        session_id: Some(room.into()), agent_id: Some(agent.into()),
        turn_id: Some("peek-turn".into()), prompt_id: Some("peek-prompt".into()),
        provider_run_id: Some(run.id().into()), ..Default::default()
    };
    let prefix = "x".repeat(75);
    let entries = [
        crate::history::SessionHistoryEntry::user_prompt(room, "attachment", agent, "test protection"),
        crate::history::SessionHistoryEntry::provider_output(room, run.id(), Some(agent),
            crate::terminal::TerminalOutputKind::ProviderOutput, Some("answer".into()), format!("{prefix}{secret}")),
    ];
    for (sequence, entry) in entries.into_iter().enumerate() {
        runtime.owned.operational_history_store.append(
            &crate::history::HistoryEvent::transcript(sequence as u64 + 1, &entry, context.clone())
        ).unwrap();
    }
    let peek = runtime.dispatch_authenticated_runtime_tool_call(TOKEN, "chariox.peek_agent", serde_json::json!({"agent":agent})).await.unwrap();
    let entry = peek.payload["entries"].as_array().unwrap().last().unwrap();
    assert!(!entry["title"].as_str().unwrap().contains("abc"), "sanitize before truncation: {}", entry["title"]);
    let page = runtime.dispatch_authenticated_runtime_tool_call(TOKEN, "chariox.read_agent_entry",
        serde_json::json!({"agent":agent,"entry_id":entry["entry_id"],"offset":75,"limit":3})
    ).await.unwrap();
    assert_eq!(page.payload["text"], "[re", "sanitize the full entry before taking a page");
    assert_eq!(page.payload["total_chars"], 85, "offsets refer to sanitized content");
}
