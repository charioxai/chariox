//! A hot reload is not the end of the kernel: a run whose process it kept authenticates its
//! tool calls (runtime tools, Vault) from the first moment, before any recovered prompt
//! claims the run. The new kernel has no run under the token yet; only the handover knows it.
use super::*;

/// The run as a hot reload hands it over: the new kernel's provider store has no live run for the
/// agent (the fixture's run is ended) and does not know the token the process calls with.
struct Surviving {
    token: String,
    run_id: String,
}

impl Drop for Surviving {
    fn drop(&mut self) {
        crate::hot_reload::forget_unclaimed_run_for_test(&self.run_id);
    }
}

async fn surviving_run(f: &Fixture) -> Surviving {
    let mut run = f.app.lock().await.providers.get_run(&f.run).unwrap();
    f.app
        .lock()
        .await
        .providers
        .mark_run_ended_provider_only(&f.room, &f.run)
        .unwrap();
    // Each fixture kernel starts its own run ID counter, but handover is process-wide.
    let surviving = Surviving {
        token: format!("handed-over-token-{}", f.room),
        run_id: format!("handed-over-{}-{}", f.room, f.run),
    };
    run.adopt_handed_over_identity(
        surviving.run_id.clone(),
        Some(surviving.token.clone()),
        None,
    );
    assert!(f
        .runtime
        .owned
        .provider_store
        .get_runs_by_runtime_mcp_auth_token(&surviving.token)
        .is_empty());
    crate::hot_reload::hold_unclaimed_run_for_tests(
        crate::hot_reload::run_after_handover_for_tests(&run),
    );
    surviving
}

#[tokio::test]
async fn a_surviving_run_calls_a_runtime_tool_before_a_prompt_claims_it() {
    let f = fixture("handed-over-runtime-tool", true).await;
    let surviving = surviving_run(&f).await;
    let task = f.begin(&f.prompt_id);
    let timer = call(
        &f,
        &surviving.token,
        "chariox.events.timer",
        &task,
        serde_json::json!({"label":"armed after the reload","delay_ms":3_600_000}),
    )
    .await
    .expect("the handed-over run's token still authenticates its runtime tools");
    assert!(timer.ok, "{:?}", timer.payload);
    assert!(
        f.store()
            .agent_wake(timer.payload["wake_id"].as_str().unwrap())
            .unwrap()
            .is_some(),
        "the timer is the kernel's, as one armed before the reload"
    );
}

#[tokio::test]
async fn a_surviving_run_calls_a_vault_tool_before_a_prompt_claims_it() {
    let f = fixture("handed-over-vault-tool", true).await;
    let surviving = surviving_run(&f).await;
    let context = f
        .runtime
        .vault_agent_context(&surviving.token)
        .expect("the Vault knows the handed-over run's agent");
    assert_eq!(context.run.id(), surviving.run_id);
    assert_eq!(context.agent_id, f.agent);
    assert_eq!(context.session_id, f.room);
    let task = f.begin(&f.prompt_id);
    let refused = call(
        &f,
        &surviving.token,
        "chariox.vault.use_env",
        &task,
        serde_json::json!({"origin":"https://example.test","argv":["true"],"env":{}}),
    )
    .await
    .err()
    .map(|error| error.to_string())
    .unwrap_or_default();
    for authentication in [
        "Vault operations require one authenticated provider run",
        "invalid runtime MCP auth token",
    ] {
        assert!(!refused.contains(authentication), "{refused}");
    }
}

#[tokio::test]
async fn a_token_nobody_holds_is_still_refused() {
    let f = fixture("handed-over-unknown-token", true).await;
    let _surviving = surviving_run(&f).await;
    let task = f.begin(&f.prompt_id);
    let refused = call(
        &f,
        "a-token-no-run-holds",
        "chariox.events.timer",
        &task,
        serde_json::json!({"label":"no","delay_ms":3_600_000}),
    )
    .await
    .expect_err("a token of no run, handed over or not, authenticates nothing");
    assert!(refused
        .to_string()
        .contains("invalid runtime MCP auth token"));
}

/// A run workflows can discover before the reload is still discoverable after it: the next
/// program reads the run through the serialized handover, which must carry the runtime-only
/// `workflow_tools_enabled` the run's own serialization leaves out.
#[tokio::test]
async fn a_run_workflows_can_discover_is_still_discoverable_after_the_handover() {
    let f = fixture("handed-over-workflow-discovery", true).await;
    f.app
        .lock()
        .await
        .providers
        .enable_workflow_tools(&f.run)
        .unwrap();
    let names = |token: &str| -> std::collections::BTreeSet<String> {
        f.runtime
            .runtime_tool_specs_for_auth_token(token)
            .into_iter()
            .map(|spec| spec.name)
            .collect()
    };
    let live = names(&runtime_token(&f).await);
    let workflow_only = crate::transport::runtime_tools::workflow_runtime_tool_specs();
    assert!(
        workflow_only.iter().all(|spec| live.contains(&spec.name)),
        "the live run discovers the workflow actions"
    );
    let surviving = surviving_run(&f).await;
    let after = names(&surviving.token);
    assert_eq!(
        after,
        live,
        "the handed-over run discovers what the live run did; differing: {:?}",
        live.symmetric_difference(&after).collect::<Vec<_>>()
    );
}

/// Every field tool calls read from a run is as the live run held it after the handover. The
/// fields the handover leaves behind (launch environment, arguments, program, launch config,
/// resume state, external import, resolved account, utility-run markers, substitute, settings
/// staleness, sync roots, project environment revision, native checkpoint, last output time)
/// are read at launch, prompt dispatch, settlement and reload: the claiming run does those.
#[tokio::test]
async fn a_handed_over_run_holds_every_field_tool_calls_read_as_the_live_run_did() {
    let f = fixture("handed-over-field-parity", true).await;
    let mut live = {
        let app = f.app.lock().await;
        let mut run = app.providers.get_run(&f.run).unwrap();
        run.set_mcp_servers_for_tests(vec![crate::mcp::CharioxMcpServerConfig::stdio(
            "docs",
            "docs-server",
            Vec::new(),
        )]);
        run.enable_workflow_tools();
        run.mark_workflow_fresh_context("workflow-node-run-1");
        run.set_remote_extension_manifest(crate::extension::RemoteExtensionManifest {
            tools: Vec::new(),
            room_browser_available: true,
        });
        run.set_control_capabilities(vec![crate::provider::ControlCapability::new(
            crate::provider::ControlOperation::AckWorkflowTurn,
            crate::provider::ControlCapabilityMode::Mcp,
        )]);
        run
    };
    live.mark_running();
    let after = crate::hot_reload::run_after_handover_for_tests(&live);
    let read = |run: &crate::provider::RuntimeProviderRun| -> Vec<String> {
        vec![
            format!("{:?}", run.id()),
            format!("{:?}", run.session_id()),
            format!("{:?}", run.agent_instance_id()),
            format!("{:?}", run.owner_user_id()),
            format!("{:?}", run.adapter_key()),
            format!("{:?}", run.provider()),
            format!("{:?}", run.account_profile()),
            format!("{:?}", run.model()),
            format!("{:?}", run.variant()),
            format!("{:?}", run.execution_mode()),
            format!("{:?}", run.permission_level()),
            format!("{:?}", run.write_access_mode()),
            format!("{:?}", run.control_capabilities()),
            format!("{:?}", run.endpoint_mode()),
            format!("{:?}", run.client_interface()),
            format!("{:?}", run.working_directory()),
            format!("{:?}", run.runtime_mcp_server_url()),
            format!("{:?}", run.runtime_mcp_auth_token()),
            format!(
                "{:?}",
                run.mcp_servers()
                    .iter()
                    .map(|server| server.name.clone())
                    .collect::<Vec<_>>()
            ),
            format!(
                "{:?}",
                run.remote_extension_manifest().room_browser_available
            ),
            format!("{:?}", run.workflow_tools_enabled()),
            format!("{:?}", run.workflow_fresh_context_node_run_id()),
            format!("{:?}", run.state()),
        ]
    };
    assert_eq!(read(&after), read(&live));
    assert!(after.workflow_tools_enabled());
    assert_eq!(
        after.workflow_fresh_context_node_run_id(),
        Some("workflow-node-run-1")
    );
}
