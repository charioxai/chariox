//! MP-08 / MP-10 / MP-11: authenticated home preparation of leased native composer turns.
use super::*;
use crate::{
    provider::{
        LaunchProviderRequest, ProviderClientInterface, ProviderLaunchResult, RuntimeProviderRun,
    },
    skill::{service::SkillCatalogService, v2::*},
    transport::relay_peer::{RelayPeerRequest, RemoteWorkspaceLiveSyncContext},
};

async fn native_home() -> (
    KernelRuntimeState,
    SkillCatalogService,
    SkillRecord,
    RemoteWorkspaceLiveSyncContext,
    String,
    String,
) {
    let (runtime, service, skill, context, session, agent) =
        super::skill_delivery_tests::manual_review_fixture().await;
    runtime
        .owned
        .agent_store
        .set_agent_runtime_profile(
            &agent,
            "claude",
            Some("haiku".into()),
            None,
            Default::default(),
        )
        .unwrap();
    let request = LaunchProviderRequest::new(&session, "claude", "claude", "default", "haiku")
        .with_agent_id(&agent)
        .with_client_interface(ProviderClientInterface::NativeTui);
    let mut run = RuntimeProviderRun::new(
        crate::provider::projected_leased_provider_run_id(
            &context.leased_agent_id,
            &context.worker_provider_run_id,
        ),
        &request,
        ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "leased-native-composer".into(),
            pty_target: None,
            pty_program: None,
            pty_args: vec![],
            pty_env: Default::default(),
            pty_env_remove: vec![],
            working_directory: None,
            structured_endpoint: None,
        },
    );
    run.mark_running();
    runtime
        .owned
        .provider_run_projection
        .update_remote_snapshot(run);
    runtime
        .owned
        .agent_store
        .set_remote_execution_active_worker_provider_run_id(&agent, None)
        .unwrap();
    (runtime, service, skill, context, session, agent)
}
async fn prepare(
    runtime: &KernelRuntimeState,
    context: &RemoteWorkspaceLiveSyncContext,
    prompt: &str,
    native: Vec<NativeSkillEntry>,
    intact: bool,
) -> serde_json::Value {
    let args = serde_json::json!({"prompt_id":"native-composer-next","prompt":prompt,"native":native,"cached_digests":[],"context_intact":intact});
    let tool = "_chariox_prepare_native_skill_turn";
    let request = RelayPeerRequest::ForwardCapabilityRuntimeTool {
        context: context.clone(),
        tool_name: tool.into(),
        arguments: args.clone(),
    };
    let peer = runtime.with_relay_peer_authority(
        crate::runtime::relay_peer_authority::test_peer_authority(&context.worker_kernel_id),
    );
    let admitted = peer
        .prepare_forwarded_peer_request(&request)
        .await
        .expect("home must prepare a leased composer turn");
    // The same live native process can become the active turn while preparation
    // crosses the relay. Recheck its identity rather than pinning the idle view.
    runtime
        .owned
        .agent_store
        .set_remote_execution_active_worker_provider_run_id(
            &context.home_agent_id,
            Some(context.worker_provider_run_id.clone()),
        )
        .unwrap();
    let (result, _) = admitted
        .dispatch_forwarded_capability_runtime_tool_call(context.clone(), tool.into(), args)
        .await
        .unwrap();
    assert!(result.ok);
    result.payload
}
fn publish(service: &SkillCatalogService, actions: Vec<SkillCatalogAction>) {
    let mutation = SkillCatalogMutation {
        scope: SkillScope::Chariox,
        expected_revision: service.storage.snapshot().unwrap().revision,
        actions,
    };
    let prepared = service.prepare(&mutation).unwrap();
    service.publish_authorized(&mutation, &prepared).unwrap();
}

#[tokio::test]
async fn leased_composer_restores_updated_manual_loads_and_auto_discovery() {
    let (runtime, service, skill, context, session, _agent) = native_home().await;
    super::skill_delivery_tests::acknowledge(&runtime,&context,serde_json::json!({"delivery":"typed","prompt_id":"typed-first","skill_id":skill.id,"digest":skill.digest})).await.unwrap();
    publish(&service,vec![SkillCatalogAction::Update {skill_id:skill.id.clone(),expected_digest:skill.digest.clone(),
        source:SkillInstallSource::Content {skill_md:"---\nname: review\ndescription: Review updated source\ndisable-model-invocation: true\n---\nNEW-COMPOSER-BODY\n".into()}},
        SkillCatalogAction::Install {source:SkillInstallSource::Content {skill_md:"---\nname: discover\ndescription: Inspect current kernel behavior\n---\nDiscovery instructions.\n".into()},collision:SkillCollisionAction::Reject,package_id:None}]);
    let result = prepare(&runtime, &context, "continue", vec![], true).await;
    let loaded = result["context"]["loaded"].as_array().unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0]["skill_id"], skill.id);
    assert_ne!(loaded[0]["digest"], skill.digest);
    assert_eq!(loaded[0]["body"], "");
    assert_eq!(
        loaded[0]["root"], "",
        "home paths must not be used as worker roots"
    );
    assert!(result["auto_rows"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["skill"]["name"] == "discover"));
    assert!(result["manifest"]["skills"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["digest"] == loaded[0]["digest"]));
    let digest = loaded[0]["digest"].as_str().unwrap();
    super::skill_delivery_tests::acknowledge(&runtime,&context,serde_json::json!({"delivery":"typed","prompt_id":"typed-updated","skill_id":skill.id,"digest":digest})).await.unwrap();
    assert_eq!(service.storage.loads(&session).unwrap().len(), 2);
    let held = prepare(&runtime, &context, "continue again", vec![], true).await;
    assert!(
        held["context"]["loaded"].as_array().unwrap().is_empty(),
        "an intact live run already holds this body"
    );
    let compacted = prepare(
        &runtime,
        &context,
        "continue after compaction",
        vec![],
        false,
    )
    .await;
    assert_eq!(compacted["context"]["loaded"][0]["digest"], digest);
    publish(
        &service,
        vec![SkillCatalogAction::SetState {
            skill_id: skill.id,
            state: Some(SkillState::Off),
        }],
    );
    let off = prepare(&runtime, &context, "continue after Off", vec![], true).await;
    assert!(off["context"]["loaded"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn leased_composer_native_selection_uses_the_real_adapter_before_receipt() {
    use crate::provider::skill_inventory::{
        NativeSkillCandidate, NativeSkillInventory, SkillInventoryBinding,
    };
    let (runtime, service, _skill, context, session, _agent) = native_home().await;
    let files = crate::test_support::TestWorktree::new("leased-composer-native");
    let path = files.path().join("worker-native/SKILL.md");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path,"---\nname: worker-native\ndescription: Inspect source\ndisable-model-invocation: true\n---\nNative instructions.\n").unwrap();
    let inventory = NativeSkillInventory::from_candidates(
        &SkillInventoryBinding {
            provider: "claude".into(),
            provider_version: "2.1.212".into(),
            account_profile: "default".into(),
            execution_id: format!(
                "{}:{}",
                context.worker_kernel_id, context.worker_provider_run_id
            ),
            cwd: files.path().into(),
            environment: Default::default(),
            project_id: Some("worker-project".into()),
            catalog_revision: 0,
            source_revision: 1,
        },
        vec![NativeSkillCandidate {
            name: "worker-native".into(),
            description: "Inspect source".into(),
            skill_file: path,
            exposure: NativeSkillExposure::Confirmed,
            supports_load_ack: false,
            observed_body_digest: None,
            discovery_error: None,
            plugin: None,
        }],
    )
    .unwrap();
    let result = prepare(
        &runtime,
        &context,
        "$worker-native inspect source",
        inventory.entries.clone(),
        true,
    )
    .await;
    let pinned: SkillTurnContext = serde_json::from_value(result["context"].clone()).unwrap();
    let delivery = crate::provider::skill_delivery::prepare_skill_delivery(
        "claude",
        "$worker-native inspect source",
        &pinned,
        &inventory,
        &[],
    )
    .unwrap();
    assert_eq!(delivery.visible_prompt, "/worker-native inspect source");
    assert_eq!(delivery.requires_native_load_ack.len(), 1);
    assert!(service.storage.loads(&session).unwrap().is_empty());
    let entry = inventory.entries[0].clone();
    let mut stale = entry.clone();
    if let SkillSelection::Native { execution_id, .. } = &mut stale.selection {
        *execution_id = "foreign-run".into();
    }
    assert!(super::skill_delivery_tests::acknowledge(
        &runtime,
        &context,
        serde_json::json!({"delivery":"typed_native","prompt_id":"native-only","entry":stale})
    )
    .await
    .is_err());
    super::skill_delivery_tests::acknowledge(
        &runtime,
        &context,
        serde_json::json!({"delivery":"typed_native","prompt_id":"native-only","entry":entry.clone()}),
    )
    .await
    .unwrap();
    let loads = service.storage.loads(&session).unwrap();
    assert_eq!(loads.len(), 1);
    assert_eq!(loads[0].reason, SkillLoadReason::ConfirmedNative);
    let SkillSelection::Native { entry_id, .. } = &entry.selection else {
        unreachable!()
    };
    assert_eq!(&loads[0].skill_id, entry_id);

    // Actual directory discovery reports no load acknowledgement. The composer
    // hook still acknowledges its native expansion for an equal Chariox copy.
    publish(&service, vec![SkillCatalogAction::Install {
        source: SkillInstallSource::Folder { path: files.path().join("worker-native").display().to_string() },
        collision: SkillCollisionAction::Reject, package_id: None,
    }]);
    let prompt = "$worker-native inspect folded source";
    let folded = prepare(&runtime, &context, prompt, inventory.entries.clone(), true).await;
    let pinned: SkillTurnContext = serde_json::from_value(folded["context"].clone()).unwrap();
    assert_eq!(pinned.loaded.len(), 1);
    assert!(matches!(pinned.loaded[0].delivery, SkillDelivery::ProvidedBy { .. }));
    let delivery = crate::provider::skill_delivery::prepare_skill_delivery(
        "claude", prompt, &pinned, &inventory, &[],
    ).unwrap();
    assert_eq!(delivery.visible_prompt, "/worker-native inspect folded source");
    assert!(!delivery.hidden_context.contains("Native instructions."));
    super::skill_delivery_tests::acknowledge(&runtime, &context, serde_json::json!({
        "delivery":"typed", "prompt_id":"native-folded", "skill_id":pinned.loaded[0].skill_id,
        "digest":pinned.loaded[0].digest,
    })).await.unwrap();
    let loads = service.storage.loads(&session).unwrap();
    assert_eq!(loads.len(), 2);
    assert_eq!(loads[1].skill_id, pinned.loaded[0].skill_id);
}
