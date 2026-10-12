//! MP-08 / MP-10 / MP-11 G08: admission precedes legacy recall lookup.
use crate::provider::{
    AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult, RuntimeProviderRun,
};

#[test]
fn mp11_g08_both_recall_encodings_deny_other_rooms() {
    std::thread::Builder::new().stack_size(64 * 1024 * 1024).spawn(|| {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
            let worktree = crate::test_support::TestWorktree::new("g08-recall-scope");
            let mut config = crate::config::DaemonConfig::for_tests().with_session_history_root(worktree.path().join("history"));
            config.room_agent_tools = false;
            let mut app = crate::DaemonApp::bootstrap(config).unwrap();
            let (room, actor) = crate::app::KernelSessionService::new(&mut app).create_session(worktree.session_request()).unwrap();
            let (foreign, _) = crate::app::KernelSessionService::new(&mut app).create_session(worktree.session_request()).unwrap();
            let request = LaunchProviderRequest::new(room.id(), "dev-stub", "dev-stub", "default", "default").with_agent_id(actor.id());
            let mut run = RuntimeProviderRun::new("g08-recall-run", &request, ProviderLaunchResult {
                endpoint_mode: AgentEndpointMode::External, process_label: "metadata-only".into(),
                pty_target: None, pty_program: None, pty_args: vec![], pty_env: Default::default(), pty_env_remove: vec![],
                working_directory: Some(worktree.path().to_owned()), structured_endpoint: None,
            });
            run.mark_running();
            run.set_runtime_mcp_auth_token(Some("g08-recall-bearer".into()));
            app.providers_mut().insert_run_for_test(run.clone());
            let router = crate::runtime::router::CommandRouter::with_interactive_capacity(std::sync::Arc::new(tokio::sync::Mutex::new(app)), 1);
            let state = router.runtime_state();
            state.owned.provider_run_projection.update(run);
            let mut failures = Vec::new();
            for name in ["chariox.search_recall", "mcp__chariox__search_recall", "mcp__chariox__chariox_search_recall", "chariox.query_recall", "mcp__chariox__query_recall", "mcp__chariox__chariox_query_recall"] {
                for scope in [None, Some("current_session"), Some("all")] {
                    let mut arguments = serde_json::json!({"query":"g08-marker", "session_id":foreign.id()});
                    if name.contains("query_recall") { arguments.as_object_mut().unwrap().remove("query"); }
                    if let Some(scope) = scope { arguments["scope"] = serde_json::json!(scope); }
                    let result = router.dispatch_authenticated_runtime_tool_call("g08-recall-bearer", name, arguments).await;
                    if !result.is_err_and(|error| matches!(error, crate::error::DaemonError::LocalTransport { operation: "runtime_tool_recall_scope", .. })) { failures.push(format!("{name} / {scope:?}")); }
                }
                let mut arguments = serde_json::json!({"query":"g08-marker", "scope":"all"});
                if name.contains("query_recall") { arguments.as_object_mut().unwrap().remove("query"); }
                // `all` without a target resolves to the caller room, never a global lookup.
                if !router.dispatch_authenticated_runtime_tool_call("g08-recall-bearer", name, arguments).await.is_ok_and(|result| result.ok) { failures.push(format!("{name} / all caller room")); }
            }
            let current = router.dispatch_authenticated_runtime_tool_call("g08-recall-bearer", "chariox.search_recall", serde_json::json!({"query":"g08-marker", "session_id":room.id()})).await.unwrap();
            assert!(current.ok, "MP-11 G08: current-room legacy recall remains usable");
            state.shutdown_cleanup().await.unwrap();
            assert!(failures.is_empty(), "MP-11 G08: recall must deny before lookup: {failures:?}");
        });
    }).unwrap().join().unwrap();
}
