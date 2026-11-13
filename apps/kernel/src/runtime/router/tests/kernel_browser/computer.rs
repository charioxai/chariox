//! MP-08 / MP-11: public MCP loading/admission checks without paid execution.
use super::*;
#[test]
fn mp11_leased_computer_requires_a_typed_explicit_desktop_grant() {
    let grant = json!({"command":{"op":"grant_leased_computer","agent_id":"agent","target":{"surface_id":"desktop","generation":"generation"}}});
    assert!(
        serde_json::from_value::<KernelBrowserRequest>(grant).is_ok(),
        "MP-08/MP-11: human must be able to grant the leased agent the observed desktop"
    );
    for value in [
        json!({"command":{"op":"grant_leased_computer","agent_id":"agent"}}),
        json!({"command":{"op":"grant_leased_computer","agent_id":"agent","target":{"surface_id":"desktop","generation":"generation"},"owner":"forged"}}),
    ] {
        assert!(serde_json::from_value::<KernelBrowserRequest>(value).is_err());
    }
}
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

#[test]
#[cfg(target_os = "linux")]
fn mp11_authenticated_computer_clipboard_read_preserves_agent_copy_while_ocr_stays_masked() {
    run_test(|| Box::pin(clipboard_read_check()));
}

async fn clipboard_read_check() {
    const CANARY: &str = "MP11-explicit-clipboard-canary";
    let workspace = crate::test_support::TestWorktree::new("culinux-computer-clipboard-read");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let (session, first) = crate::app::KernelSessionService::new(&mut app)
        .create_session(workspace.session_request())
        .unwrap();
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
    let root = std::env::temp_dir().join(format!(
        "chariox-computer-clipboard-{:032x}",
        rand::random::<u128>()
    ));
    std::fs::create_dir(&root).unwrap();
    let script = root.join("computer-controller.sh");
    std::fs::write(&script, r##"set -eu
while IFS= read -r request; do
 id=${request#*:}; id=${id%%,*}
 case "$request" in
  *'"method":"health"'*) printf '{"id":%s,"ok":true,"result":{"state":"ready","process_id":%s,"diagnostic_code":null}}\n' "$id" "$$" ;;
  *'"method":"host.protect"'*) printf '{"id":%s,"ok":true,"result":{}}\n' "$id" ;;
  *'"method":"host.revoke_subscriptions"'*) printf '{"id":%s,"ok":true,"result":{}}\n' "$id" ;;
  *'"method":"host.downloads.cleanup"'*) printf '{"id":%s,"ok":true,"result":{}}\n' "$id" ;;
  *'"method":"host.computer.retire"'*) printf '{"id":%s,"ok":true,"result":{}}\n' "$id" ;;
  *'"method":"host.browser.retire_input"'*) printf '{"id":%s,"ok":true,"result":{}}\n' "$id" ;;
  *'"method":"host.browser"'*'"op":"start"'*) printf '{"id":%s,"ok":true,"result":{"generation":1,"tabs":[{"tab_id":"fixture-tab","document_id":"fixture-document"}]}}\n' "$id" ;;
  *'"method":"host.browser"'*'"op":"state"'*) printf '{"id":%s,"ok":true,"result":{"generation":1,"tabs":[{"tab_id":"fixture-tab","document_id":"fixture-document"}]}}\n' "$id" ;;
  *'"method":"host.computer"'*'"op":"state"'*) printf '{"id":%s,"ok":true,"result":{"surface_id":"fixture-surface","generation":"fixture-generation"}}\n' "$id" ;;
  *'"method":"host.computer"'*'"op":"clipboard_read"'*) printf '{"id":%s,"ok":true,"result":{"text":"MP11-explicit-clipboard-canary"}}\n' "$id" ;;
  *'"method":"host.computer"'*'"op":"ocr"'*) printf '{"id":%s,"ok":true,"result":{"text":"MP11-explicit-clipboard-canary"}}\n' "$id" ;;
  *'"method":"shutdown"'*) printf '{"id":%s,"ok":true,"result":{"state":"stopped","process_id":null,"diagnostic_code":null}}\n' "$id"; exit 0 ;;
 esac
done
"##).unwrap();
    router
        .runtime_state()
        .install_kernel_computer_fixture(first.owner_user_id(), &script, &root);
    router
        .runtime_state()
        .register_kernel_room_fixture_value(session.id(), CANARY);
    router
        .runtime_state()
        .register_kernel_browser_fixture_value(first.owner_user_id(), CANARY);
    focus(&router, session.id(), first.id()).await;
    router
        .dispatch_authenticated_runtime_tool_call(token, "chariox.load_kernel_computer", json!({}))
        .await
        .unwrap();

    let target = json!({"surface_id":"fixture-surface","generation":"fixture-generation"});
    let clipboard = router
        .dispatch_authenticated_runtime_tool_call(
            token,
            "chariox.kernel_computer",
            json!({"command":{"op":"clipboard_read","target":target}}),
        )
        .await
        .unwrap()
        .payload;
    assert_eq!(
        clipboard["text"], CANARY,
        "agent's explicit clipboard read is preserved"
    );

    let ocr = router
        .dispatch_authenticated_runtime_tool_call(
            token,
            "chariox.kernel_computer",
            json!({"command":{"op":"ocr","target":target}}),
        )
        .await
        .unwrap()
        .payload;
    assert_eq!(
        ocr["text"], "[redacted]",
        "observation output remains scrubbed"
    );

    assert!(
        router
            .dispatch_authenticated_runtime_tool_call(
                token,
                "chariox.kernel_computer",
                json!({"command":{"op":"clipboard_read","target":target,"observed_by":"forged"}}),
            )
            .await
            .is_err(),
        "malformed clipboard commands remain rejected"
    );
    let revoke = LocalDaemonRequest::KernelBrowser(KernelBrowserRequest {
        command: KernelBrowserCommand::RevokeGrants {
            agent_id: Some(first.id().into()),
        },
    });
    router
        .dispatch(
            terminal_command("computer-clipboard-revoke", &revoke),
            revoke,
        )
        .await
        .unwrap();
    assert!(
        router
            .dispatch_authenticated_runtime_tool_call(
                token,
                "chariox.kernel_computer",
                json!({"command":{"op":"clipboard_read","target":target}}),
            )
            .await
            .is_err(),
        "a revoked agent cannot read the clipboard"
    );

    router.runtime_state.shutdown_cleanup().await.unwrap();
    std::fs::remove_dir_all(root).unwrap();
}
