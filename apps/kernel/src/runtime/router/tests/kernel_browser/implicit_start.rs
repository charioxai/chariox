//! An agent that holds the browser grant reads a stopped browser: the first read starts it once.
use super::*;

async fn tool(
    router: &CommandRouter,
    token: &str,
    arguments: Value,
) -> Result<crate::transport::runtime_tools::RuntimeToolResult, crate::DaemonError> {
    router
        .dispatch_authenticated_runtime_tool_call(token, "chariox.kernel_browser", arguments)
        .await
}

fn starts(root: &std::path::Path) -> usize {
    std::fs::read_to_string(root.join("starts")).map_or(0, |text| text.lines().count())
}

#[test]
fn an_agent_read_starts_a_never_started_browser_once_and_a_stop_stays_stopped() {
    run_test(|| {
        Box::pin(async {
            let workspace = crate::test_support::TestWorktree::new("shotfix-implicit-start");
            let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
            let (session, agent) = crate::app::KernelSessionService::new(&mut app)
                .create_session(workspace.session_request())
                .unwrap();
            let run = launch_test_provider(
                &mut app,
                session.id(),
                agent.id(),
                "dev-stub",
                "dev-stub",
                "default",
            );
            let token = run.runtime_mcp_auth_token().unwrap();
            let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
            let root = std::env::temp_dir().join(format!(
                "chariox-shotfix-implicit-{:032x}",
                rand::random::<u128>()
            ));
            std::fs::create_dir(&root).unwrap();
            router
                .runtime_state()
                .install_unstarted_kernel_browser_fixture(agent.owner_user_id(), &root);
            focus(&router, session.id(), agent.id()).await;
            router
                .dispatch_authenticated_runtime_tool_call(
                    token,
                    "chariox.load_kernel_browser",
                    json!({}),
                )
                .await
                .unwrap();
            assert_eq!(starts(&root), 0);

            let first = tool(&router, token, json!({"command":{"op":"state"}}))
                .await
                .expect("the first read starts the browser");
            assert_eq!(first.payload["tabs"][0]["tab_id"], "host-tab-fixture");
            assert_eq!(starts(&root), 1, "started once");
            tool(&router, token, json!({"command":{"op":"state"}}))
                .await
                .unwrap();
            assert_eq!(starts(&root), 1, "a running browser is not started again");

            // A stop is the person's (or agent's) decision: reads do not undo it.
            tool(&router, token, json!({"command":{"op":"stop"}}))
                .await
                .unwrap();
            for _ in 0..2 {
                let error = tool(&router, token, json!({"command":{"op":"state"}}))
                    .await
                    .unwrap_err()
                    .to_string();
                assert!(error.contains("explicitly start"), "{error}");
            }
            assert_eq!(starts(&root), 1);
            tool(&router, token, json!({"command":{"op":"start"}}))
                .await
                .unwrap();
            assert_eq!(starts(&root), 2);
            tool(&router, token, json!({"command":{"op":"state"}}))
                .await
                .unwrap();
            router.runtime_state().shutdown_cleanup().await.unwrap();
            std::fs::remove_dir_all(root).unwrap();
        })
    });
}
