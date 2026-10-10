//! MP-08 / MP-11: public MCP loading/admission checks without paid execution.
use super::*;
#[test]
#[cfg(target_os = "linux")]
fn mp11_kernel_computer_mcp_loader_revoke_and_human_channel_guards() {
    run_test(|| Box::pin(check()));
}
async fn check() {
    let workspace = crate::test_support::TestWorktree::new("culinux-computer-mcp");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let (session, first) = crate::app::KernelSessionService::new(&mut app)
        .create_session(workspace.session_request())
        .unwrap();
    let second = spawn_test_agent(&mut app, session.id(), "second", "dev-stub");
    let run = launch_test_provider(
        &mut app,
        session.id(),
        first.id(),
        "dev-stub",
        "dev-stub",
        "default",
    );
    let token = run.runtime_mcp_auth_token().unwrap();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    let names = || {
        router
            .runtime_tool_specs_for_auth_token(token)
            .into_iter()
            .map(|t| t.name)
            .collect::<Vec<_>>()
    };
    focus(&router, session.id(), second.id()).await;
    assert!(router
        .dispatch_authenticated_runtime_tool_call(token, "chariox.load_kernel_computer", json!({}))
        .await
        .is_err());
    focus(&router, session.id(), first.id()).await;
    assert!(names().contains(&"chariox.load_kernel_computer".into()));
    assert!(!names().contains(&"chariox.kernel_computer".into()));
    router
        .dispatch_authenticated_runtime_tool_call(token, "chariox.load_kernel_computer", json!({}))
        .await
        .unwrap();
    assert!(names().contains(&"chariox.kernel_computer".into()));
    let target = json!({"surface_id":"s","generation":"g"});
    for command in [
        json!({"op":"takeover","target":target}),
        json!({"op":"release","target":target}),
        json!({"op":"actors"}),
    ] {
        let error = router
            .dispatch_authenticated_runtime_tool_call(
                token,
                "chariox.kernel_computer",
                json!({"command":command}),
            )
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("human desktop channel"));
    }
    assert!(router
        .dispatch_authenticated_runtime_tool_call(
            token,
            "chariox.kernel_computer",
            json!({"command":{"op":"state","observed_by":"forged"}})
        )
        .await
        .is_err());
    // Browser loading does not permit smuggling the new native contract.
    router
        .dispatch_authenticated_runtime_tool_call(token, "chariox.load_kernel_browser", json!({}))
        .await
        .unwrap();
    assert!(router
        .dispatch_authenticated_runtime_tool_call(
            token,
            "chariox.kernel_browser",
            json!({"command":{"op":"computer","command":{"op":"state"}}})
        )
        .await
        .is_err());
    let request = LocalDaemonRequest::KernelBrowser(KernelBrowserRequest {
        command: KernelBrowserCommand::RevokeGrants {
            agent_id: Some(first.id().into()),
        },
    });
    router
        .dispatch(terminal_command("culinux-revoke", &request), request)
        .await
        .unwrap();
    assert!(!names().contains(&"chariox.kernel_computer".into()));
    let error = router
        .dispatch_authenticated_runtime_tool_call(
            token,
            "chariox.kernel_computer",
            json!({"command":{"op":"state"}}),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(
            error,
            crate::error::DaemonError::UserDomainRefused {
                reason: crate::error::UserDomainRefusalReason::NotGranted
            }
        ),
        "{error}"
    );
    router.runtime_state.shutdown_cleanup().await.unwrap();
}
