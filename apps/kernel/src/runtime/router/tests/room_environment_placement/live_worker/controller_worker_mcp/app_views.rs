use super::*;

#[test]
fn leased_provider_discovers_and_invokes_home_app_view_tools() {
    crate::test_support::isolated_env_test!();
    run_test(check_leased_app_view_tools);
}

async fn check_leased_app_view_tools() {
    let _tools = ScopedEnvironment::set([("CHARIOX_ROOM_AGENT_TOOLS", "1".into())]);
    let mut fixture = LiveWorker::start().await;
    let (_worker_state, worker) = start_agent_worker(&mut fixture).await;
    let check = std::panic::AssertUnwindSafe(async {
        wait_for_agent_worker(&fixture).await;
        let leased =
            launch_leased_room_provider(&fixture, &worker, "inspect the open App view").await;
        let token = worker
            .runtime_state
            .runtime_mcp_auth_token_for_provider_run(&leased.worker_provider_run_id)
            .unwrap();
        let owner = {
            let app = fixture.home.app.lock().await;
            app.agents()
                .get_agent(&leased.home_agent_id)
                .unwrap()
                .owner_user_id()
                .to_string()
        };
        {
            let app = fixture.home.app.lock().await;
            crate::durable_state::app_state::fixture_inbox_installation(
                &app.durable_state_store(),
                &owner,
            );
        }
        assert!(!worker
            .runtime_state
            .runtime_tool_specs_for_auth_token(&token)
            .iter()
            .any(|spec| spec.name == "chariox.app_view_catalog"));
        fixture
            .home
            .runtime_state
            .grant_agent_extension(
                &leased.home_agent_id,
                crate::extension::ExtensionGrant::app("installed"),
                &owner,
            )
            .await
            .unwrap();
        let names = worker
            .runtime_state
            .runtime_tool_specs_for_auth_token(&token)
            .into_iter()
            .map(|spec| spec.name)
            .collect::<std::collections::BTreeSet<_>>();
        for name in ["catalog", "invoke", "state", "changes", "batch"] {
            assert!(
                names.contains(&format!("chariox.app_view_{name}")),
                "missing App view tool {name}"
            );
        }
        fixture.home.runtime_state.app_control().views().register(
            &fixture.rooms[0],
            "home-only-view",
            crate::runtime::app_views::AppViewBinding {
                owner: owner.clone(),
                installation: "installed".into(),
                generation: 3,
                panel: Default::default(),
                logical_tab: None,
            },
        );
        let result = worker
            .runtime_state
            .dispatch_authenticated_runtime_tool_call(&token, "chariox.app_view_catalog", json!({}))
            .await
            .unwrap();
        assert!(result.ok, "{:?}", result.payload);
        assert_eq!(result.payload["views"][0]["view"], "home-only-view");
        let result = worker
            .runtime_state
            .dispatch_authenticated_runtime_tool_call(
                &token,
                "chariox.app_view_invoke",
                json!({"view":"missing-view", "action":"save"}),
            )
            .await
            .unwrap();
        assert!(!result.ok);
        assert_eq!(result.payload["code"], "unknown_view");
        fixture
            .home
            .runtime_state
            .revoke_agent_extension(
                &leased.home_agent_id,
                crate::extension::ExtensionKind::App,
                "installed",
                &owner,
            )
            .await
            .unwrap();
        assert!(worker
            .runtime_state
            .dispatch_authenticated_runtime_tool_call(&token, "chariox.app_view_catalog", json!({}))
            .await
            .is_err());
    })
    .catch_unwind()
    .await;
    let cleanup = worker
        .app
        .lock()
        .await
        .teardown_provider_processes(Some("managed-dev-stub"), true);
    fixture.stop().await;
    cleanup.expect("stop provider after App view tool checks");
    if let Err(panic) = check {
        std::panic::resume_unwind(panic);
    }
}
