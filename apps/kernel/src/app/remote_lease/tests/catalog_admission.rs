use super::*;
use crate::extension::{
    ExtensionAuthority, ExtensionDefinitionOrigin, ExtensionExecutionLocation, ExtensionKind,
    RemoteExtensionManifest, RemoteExtensionTool,
};
use crate::provider::{LaunchProviderRequest, RuntimeProviderRun};

fn catalog(revision: &str) -> RemoteExtensionManifest {
    RemoteExtensionManifest {
        tools: vec![RemoteExtensionTool {
            kind: ExtensionKind::App,
            name: "installed".into(),
            tool_name: "app_catalog_fixture_echo".into(),
            description: "Echo fixture".into(),
            input_schema: serde_json::json!({"type":"object"}),
            authority: ExtensionAuthority::Home,
            definition_origin: ExtensionDefinitionOrigin::Home,
            execution_location: ExtensionExecutionLocation::Home,
            safety: None,
            timeout_sec: Some(30),
            version_hash: Some(revision.into()),
        }],
    }
}

fn start_fixture_run(
    app: &mut DaemonApp,
    lease: &LeasedAgent,
    manifest: RemoteExtensionManifest,
) -> RuntimeProviderRun {
    app.providers_mut()
        .launch_run_detached(
            LaunchProviderRequest::new(
                &lease.backing_session_id,
                "dev-stub",
                &lease.provider,
                &lease.account_profile,
                lease.model.as_deref().unwrap_or("default"),
            )
            .with_agent_id(&lease.backing_agent_id)
            .with_variant(lease.effort.clone())
            .with_remote_extension_manifest(manifest),
        )
        .unwrap()
}

#[tokio::test]
async fn leased_catalog_change_returns_the_new_run_through_existing_prompt_admission() {
    for (original, desired) in [
        (RemoteExtensionManifest::default(), catalog("1:first")),
        (catalog("1:first"), catalog("2:replacement")),
        (catalog("1:first"), RemoteExtensionManifest::default()),
    ] {
        let (mut app, lease) = leased_agent_fixture(false);
        let profile = crate::transport::relay_peer::RelayAgentExecutionProfile::from(
            &app.agents().get_agent(&lease.backing_agent_id).unwrap(),
        );
        let previous = start_fixture_run(&mut app, &lease, original);
        // The ordinary manifest sync is only metadata. It cannot certify that
        // the old provider has refreshed tools/list, including on revocation.
        RemoteLeaseRuntime::new(&mut app)
            .update_leased_agent_remote_extension_manifest(&lease.id, desired.clone())
            .unwrap();
        let metadata_updated = app.providers().get_run(previous.id()).unwrap();
        assert_eq!(metadata_updated.remote_extension_manifest(), &desired);
        assert!(!metadata_updated.remote_extension_catalog_matches_launch(&desired));
        let app = std::sync::Arc::new(tokio::sync::Mutex::new(app));
        let router =
            crate::runtime::router::CommandRouter::with_interactive_capacity(app.clone(), 2);
        let (admitted_run_id, _) = router
            .relay_submit_leased_prompt(
                &lease.id,
                profile,
                "use the newly bound App catalog",
                "",
                Vec::new(),
                None,
                None,
                Vec::new(),
                None,
                desired.clone(),
            )
            .await
            .unwrap();
        let app = app.lock().await;
        let admitted = app.providers().get_run(&admitted_run_id).unwrap();
        assert_ne!(admitted_run_id, previous.id());
        assert_eq!(admitted.remote_extension_manifest(), &desired);
        assert!(admitted.remote_extension_catalog_matches_launch(&desired));
        assert_eq!(
            app.providers().get_run(previous.id()).unwrap().state(),
            ProviderRunState::Ended
        );
    }
}

#[test]
fn changed_catalog_does_not_queue_a_prompt_on_the_busy_old_provider() {
    let (mut app, lease) = leased_agent_fixture(false);
    let original = catalog("1:first");
    let desired = catalog("2:replacement");
    let previous = start_fixture_run(&mut app, &lease, original);
    sync_active_prompt(&mut app, &lease);
    RemoteLeaseRuntime::new(&mut app)
        .update_leased_agent_remote_extension_manifest(&lease.id, desired.clone())
        .unwrap();
    let result = RemoteLeaseRuntime::new(&mut app).prepare_leased_provider_run_matches_mcps(
        &lease,
        &[],
        &desired,
        false,
        false,
        false,
    );
    assert!(matches!(
        result,
        Err(DaemonError::LocalTransport {
            operation: "remote runtime tool catalog reload",
            ..
        })
    ));
    assert_eq!(
        app.providers().get_run(previous.id()).unwrap().state(),
        ProviderRunState::Running
    );
    assert_eq!(
        app.prompt_owner_active_prompt_for_agent(
            &lease.backing_session_id,
            &lease.backing_agent_id
        )
        .unwrap()
        .unwrap()
        .id(),
        "active-prompt"
    );
    assert!(app
        .prompt_owner_peek_next_queued_prompt(&lease.backing_session_id, &lease.backing_agent_id)
        .unwrap()
        .is_none());
}

#[test]
fn unchanged_catalog_keeps_the_existing_busy_provider() {
    let (mut app, lease) = leased_agent_fixture(false);
    let desired = catalog("1:first");
    let previous = start_fixture_run(&mut app, &lease, desired.clone());
    sync_active_prompt(&mut app, &lease);
    let result = RemoteLeaseRuntime::new(&mut app).prepare_leased_provider_run_matches_mcps(
        &lease,
        &[],
        &desired,
        false,
        false,
        false,
    );
    assert!(matches!(
        result,
        Ok(super::super::provider_run::LeasedProviderRunMatch::Ready(id)) if id == previous.id()
    ));
}

#[test]
fn serialized_projection_cannot_supply_a_provider_launch_cache_snapshot() {
    let (mut app, lease) = leased_agent_fixture(false);
    let desired = catalog("1:first");
    let run = start_fixture_run(&mut app, &lease, desired.clone());
    let mut serialized = serde_json::to_value(&run).unwrap();
    assert!(serialized
        .get("launched_remote_extension_manifest_hash")
        .is_none());
    serialized["launched_remote_extension_manifest_hash"] =
        serde_json::json!(desired.manifest_hash());
    assert!(serialized
        .get("observed_remote_extension_manifest_hash")
        .is_none());
    serialized["observed_remote_extension_manifest_hash"] =
        serde_json::json!(desired.manifest_hash());
    let projected: RuntimeProviderRun = serde_json::from_value(serialized).unwrap();
    assert!(!projected.remote_extension_catalog_matches_launch(&desired));
    assert!(run.remote_extension_catalog_matches_launch(&desired));
}

#[test]
fn native_catalog_waits_for_observed_refresh_and_keeps_the_attached_run_identity() {
    for original in [RemoteExtensionManifest::default(), catalog("1:first")] {
        let (mut app, lease) = leased_agent_fixture(false);
        let desired = if original.is_empty() {
            catalog("2:new")
        } else {
            RemoteExtensionManifest::default()
        };
        let native = app
            .providers_mut()
            .launch_run_detached(
                LaunchProviderRequest::new(
                    &lease.backing_session_id,
                    "dev-stub",
                    &lease.provider,
                    &lease.account_profile,
                    lease.model.as_deref().unwrap_or("default"),
                )
                .with_agent_id(&lease.backing_agent_id)
                .with_variant(lease.effort.clone())
                .with_client_interface(crate::provider::ProviderClientInterface::NativeTui)
                .with_remote_extension_manifest(original),
            )
            .unwrap();
        RemoteLeaseRuntime::new(&mut app)
            .update_leased_agent_remote_extension_manifest(&lease.id, desired.clone())
            .unwrap();
        let prepared = RemoteLeaseRuntime::new(&mut app).prepare_leased_provider_run_matches_mcps(
            &lease,
            &[],
            &desired,
            false,
            false,
            false,
        );
        assert!(matches!(
            prepared,
            Err(DaemonError::LocalTransport {
                operation: "remote runtime tool catalog reload",
                ..
            })
        ));
        assert_eq!(
            app.providers().get_run(native.id()).unwrap().state(),
            ProviderRunState::Running
        );
        assert!(app
            .providers()
            .observe_runtime_tool_catalog(native.id(), "wrong-desired-hash")
            .unwrap()
            .is_none());
        // Only the native refresher calls this store operation after the exact
        // generation's authenticated tools/list. Here test its cache transition.
        let observed = app
            .providers()
            .observe_runtime_tool_catalog(native.id(), &desired.manifest_hash())
            .unwrap()
            .unwrap();
        assert_eq!(
            observed.client_interface(),
            crate::provider::ProviderClientInterface::NativeTui
        );
        let prepared = RemoteLeaseRuntime::new(&mut app).prepare_leased_provider_run_matches_mcps(
            &lease,
            &[],
            &desired,
            false,
            false,
            false,
        );
        assert!(
            matches!(prepared, Ok(super::super::provider_run::LeasedProviderRunMatch::Ready(id)) if id == native.id())
        );
    }
}
