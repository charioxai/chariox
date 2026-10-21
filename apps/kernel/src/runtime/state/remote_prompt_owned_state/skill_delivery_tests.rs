//! MP-08 / MP-10 / MP-11: prompt ownership and durable receipt commits.
use super::*;
use crate::skill::{storage::SkillCatalogStorage, turn_authority::SkillTurnAuthority, v2::*};
use crate::transport::relay_peer::{RelayPeerRequest, RemoteWorkspaceLiveSyncContext};

async fn fixture() -> (
    KernelRuntimeState,
    RemoteWorkspaceLiveSyncContext,
    SkillCatalogStorage,
    String,
) {
    let (runtime, session_id, agent_id, attachment, binding) =
        super::tests::mp11_fixture(crate::DaemonConfig::for_tests()).await;
    let submission = runtime
        .owned
        .submit_remote_prepared_prompt(&crate::app::KernelPreparedPromptSubmission {
            session_id: session_id.clone(),
            prompt: crate::session::PromptQueueItem::new(
                "receipt-turn",
                &attachment,
                &agent_id,
                "use instructions",
                crate::session::PromptStatus::Queued,
            ),
            force_queue: false,
            refresh_projection: true,
        })
        .unwrap()
        .unwrap();
    let dispatch = submission.remote_dispatch.unwrap();
    runtime
        .owned
        .settle_remote_dispatch_if_current(&dispatch, Some("worker-run-1"))
        .unwrap();
    let session = runtime
        .owned
        .session_store
        .get_session(&session_id)
        .unwrap();
    let prompt = runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &agent_id)
        .unwrap();
    let run_id = super::super::skills_dispatch::leased_run_id(&binding.leased_agent_id);
    let record = |skill: &str, reason| SkillLoadRecord {
        skill_id: skill.into(),
        digest: "a".repeat(64),
        session_id: session_id.clone(),
        agent_id: agent_id.clone(),
        run_id: run_id.clone(),
        prompt_id: prompt.id().into(),
        reason,
        context_generation: "generation".into(),
        delivered_at_ms: 0,
    };
    let pending_loads = vec![
        record("explicit", SkillLoadReason::Explicit),
        record("model", SkillLoadReason::Model),
    ];
    let authority = SkillTurnAuthority {
        catalog_revision: 0,
        project_id: session.project_id().into(),
        provider_run_id: run_id,
        context_generation: "generation".into(),
        explicit_skill_ids: vec!["explicit".into()],
        body_budget_tokens: 100,
        reserved_body_tokens: Default::default(),
        pending_loads,
        prompt_delivery_skill_ids: ["explicit".into()].into(),
        remote_roots: Default::default(),
    };
    runtime
        .owned
        .prompt_state_owner
        .update_skill_authority_durably(
            &runtime.owned.session_store,
            &runtime.owned.durable_state_store,
            &session_id,
            &agent_id,
            prompt.id(),
            |_| Ok(authority),
        )
        .unwrap();
    let config = runtime.owned.config_projection.snapshot();
    let storage = SkillCatalogStorage::new(
        config.user_config_path.parent().unwrap(),
        runtime.owned.durable_state_store.clone(),
    )
    .unwrap();
    let context = RemoteWorkspaceLiveSyncContext {
        home_kernel_id: config.daemon_id.clone(),
        home_session_id: session_id,
        home_agent_id: agent_id,
        home_prompt_id: Some(prompt.id().into()),
        leased_agent_id: binding.leased_agent_id,
        worker_kernel_id: binding.worker_kernel_id,
        worker_machine_id: binding.worker_machine_id,
        worker_provider_run_id: "worker-run-1".into(),
        worker_worktree_path: session.worktree_id().into(),
        worker_workspace_identity: crate::io::WorkspaceIdentity::local("fixture"),
    };
    (runtime, context, storage, attachment)
}

async fn acknowledge(
    runtime: &KernelRuntimeState,
    context: &RemoteWorkspaceLiveSyncContext,
    arguments: serde_json::Value,
) -> Result<(), DaemonError> {
    let tool_name = "_chariox_skill_delivery_ack";
    let request = RelayPeerRequest::ForwardCapabilityRuntimeTool {
        context: context.clone(),
        tool_name: tool_name.into(),
        arguments: arguments.clone(),
    };
    let admitted = runtime
        .with_relay_peer_authority(crate::runtime::relay_peer_authority::test_peer_authority(
            &context.worker_kernel_id,
        ))
        .prepare_forwarded_peer_request(&request)
        .await?;
    let (result, _) = admitted
        .dispatch_forwarded_capability_runtime_tool_call(
            context.clone(),
            tool_name.into(),
            arguments,
        )
        .await?;
    assert!(result.ok);
    Ok(())
}

#[tokio::test]
async fn leased_skill_native_duplicate_retains_manual_receipt_and_next_turn_load() {
    use crate::skill::service::SkillCatalogService;
    let (runtime, session_id, agent_id, attachment, binding) =
        super::tests::mp11_fixture(crate::DaemonConfig::for_tests()).await;
    let config = runtime.owned.config_projection.snapshot();
    let storage = SkillCatalogStorage::new(
        config.user_config_path.parent().unwrap(),
        runtime.owned.durable_state_store.clone(),
    )
    .unwrap();
    let service = SkillCatalogService::new(storage.clone());
    let mutation = SkillCatalogMutation {
        scope: SkillScope::Chariox, expected_revision: 0,
        actions: vec![SkillCatalogAction::Install {
            source: SkillInstallSource::Content { skill_md: "---\nname: review\ndescription: Review safely\ndisable-model-invocation: true\n---\nReview source.\n".into() },
            collision: SkillCollisionAction::Reject, package_id: None,
        }],
    };
    let prepared = service.prepare(&mutation).unwrap();
    let catalog = service.publish_authorized(&mutation, &prepared).unwrap();
    let skill = &catalog.catalogs[0].skills[0];
    let selection = SkillSelection::Chariox {
        skill_id: skill.id.clone(),
        name: skill.name.clone(),
        catalog_revision: catalog.revision,
    };
    let native = NativeSkillEntry {
        selection: SkillSelection::Native {
            entry_id: "native-review".into(),
            name: skill.name.clone(),
            provider: "codex".into(),
            account_profile: "account".into(),
            execution_id: "worker-run-1".into(),
            inventory_revision: "native-revision".into(),
        },
        description: skill.description.clone(),
        digest: Some(skill.digest.clone()),
        exposure: NativeSkillExposure::Confirmed,
        default_state: SkillState::Manual,
        complete_manifest: true,
        supports_load_ack: true,
        discovery_error: None,
    };
    let submission = runtime
        .owned
        .submit_remote_prepared_prompt(&crate::app::KernelPreparedPromptSubmission {
            session_id: session_id.clone(),
            prompt: crate::session::PromptQueueItem::new(
                "native-manual-turn",
                &attachment,
                &agent_id,
                "$review inspect source",
                crate::session::PromptStatus::Queued,
            )
            .with_skill_selections(vec![selection])
            .with_user_skill_syntax(true),
            force_queue: false,
            refresh_projection: true,
        })
        .unwrap()
        .unwrap();
    let dispatch = submission.remote_dispatch.unwrap();
    runtime
        .owned
        .settle_remote_dispatch_if_current(&dispatch, Some("worker-run-1"))
        .unwrap();
    let session = runtime
        .owned
        .session_store
        .get_session(&session_id)
        .unwrap();
    let prompt = runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &agent_id)
        .unwrap();
    let remote = super::super::skills_dispatch::prepare_remote_turn(
        &runtime,
        &session_id,
        &agent_id,
        prompt.id(),
        &binding.leased_agent_id,
        &[native.clone()],
    )
    .unwrap()
    .unwrap();
    assert!(remote.manifest.skills.is_empty());
    assert!(remote.packages.is_empty());
    assert_eq!(remote.native_selections, vec![native.selection.clone()]);
    let prompt = runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &agent_id)
        .unwrap();
    let authority = prompt.skill_turn_authority().unwrap();
    assert_eq!(authority.explicit_skill_ids, vec![skill.id.clone()]);
    assert_eq!(authority.pending_loads.len(), 1);
    assert_eq!(authority.pending_loads[0].digest, skill.digest);
    assert!(storage.loads(&session_id).unwrap().is_empty());
    let context = RemoteWorkspaceLiveSyncContext {
        home_kernel_id: config.daemon_id.clone(),
        home_session_id: session_id.clone(),
        home_agent_id: agent_id.clone(),
        home_prompt_id: Some(prompt.id().into()),
        leased_agent_id: binding.leased_agent_id,
        worker_kernel_id: binding.worker_kernel_id,
        worker_machine_id: binding.worker_machine_id,
        worker_provider_run_id: "worker-run-1".into(),
        worker_worktree_path: session.worktree_id().into(),
        worker_workspace_identity: crate::io::WorkspaceIdentity::local("fixture"),
    };
    acknowledge(&runtime, &context, serde_json::json!({"delivery":"prompt"}))
        .await
        .unwrap();
    assert_eq!(storage.loads(&session_id).unwrap().len(), 1);
    let next = service
        .pinned_turn(
            Some(session.project_id()),
            &agent_id,
            &session_id,
            "continue reviewing",
            &[],
            &[native.clone()],
            None,
            1.0,
            8192,
            "later-generation",
        )
        .unwrap();
    assert_eq!(next.loaded.len(), 1);
    assert_eq!(next.loaded[0].skill_id, skill.id);
    assert!(next.loaded[0].body.is_empty());
    assert_eq!(next.native_selections, vec![native.selection]);
}

#[tokio::test]
async fn leased_skill_receipts_wait_for_delivery_and_bind_tool_digest() {
    let (runtime, context, storage, _) = fixture().await;
    assert!(
        storage.loads(&context.home_session_id).unwrap().is_empty(),
        "admission alone is not Loaded"
    );
    acknowledge(&runtime, &context, serde_json::json!({"delivery":"prompt"}))
        .await
        .unwrap();
    assert_eq!(
        storage
            .loads(&context.home_session_id)
            .unwrap()
            .iter()
            .map(|r| r.skill_id.as_str())
            .collect::<Vec<_>>(),
        ["explicit"]
    );
    acknowledge(
        &runtime,
        &context,
        serde_json::json!({"delivery":"tool", "skill_id":"model", "digest":"b".repeat(64)}),
    )
    .await
    .unwrap();
    assert_eq!(
        storage.loads(&context.home_session_id).unwrap().len(),
        1,
        "foreign digest is never a receipt"
    );
    for _ in 0..2 {
        acknowledge(
            &runtime,
            &context,
            serde_json::json!({"delivery":"tool", "skill_id":"model", "digest":"a".repeat(64)}),
        )
        .await
        .unwrap();
    }
    let loads = storage.loads(&context.home_session_id).unwrap();
    assert_eq!(loads.len(), 2, "delivery replay is idempotent");
    assert!(loads.iter().all(
        |r| r.prompt_id == context.home_prompt_id.as_deref().unwrap() && r.delivered_at_ms > 0
    ));
}

#[tokio::test]
async fn leased_skill_cancelled_or_lost_tool_response_leaves_no_loaded_history() {
    let (runtime, context, storage, attachment) = fixture().await;
    assert!(storage.loads(&context.home_session_id).unwrap().is_empty());
    runtime
        .owned
        .begin_remote_prompt_cancellation(
            &context.home_session_id,
            &context.home_agent_id,
            &attachment,
        )
        .unwrap();
    assert!(
        acknowledge(&runtime, &context, serde_json::json!({"delivery":"prompt"}))
            .await
            .is_err()
    );
    assert!(acknowledge(
        &runtime,
        &context,
        serde_json::json!({"delivery":"tool", "skill_id":"model", "digest":"a".repeat(64)})
    )
    .await
    .is_err());
    assert!(storage.loads(&context.home_session_id).unwrap().is_empty());
}

// MP-08 / MP-10 / MP-11: a skill-less leased turn needs no worker inventory request.
#[tokio::test]
async fn leased_turn_requests_worker_inventory_only_when_it_can_use_it() {
    use crate::skill::service::SkillCatalogService;
    let (runtime, session_id, agent_id, attachment, _binding) =
        super::tests::mp11_fixture(crate::DaemonConfig::for_tests()).await;
    let submit = |id: &str, text: &str| {
        runtime
            .owned
            .submit_remote_prepared_prompt(&crate::app::KernelPreparedPromptSubmission {
                session_id: session_id.clone(),
                prompt: crate::session::PromptQueueItem::new(
                    id,
                    &attachment,
                    &agent_id,
                    text,
                    crate::session::PromptStatus::Queued,
                )
                .with_user_skill_syntax(true),
                force_queue: false,
                refresh_projection: true,
            })
            .unwrap()
            .unwrap()
    };
    let needs = |prompt_id: &str| {
        super::super::skills_dispatch::remote_turn_needs_native_inventory(
            &runtime,
            &session_id,
            &agent_id,
            prompt_id,
        )
        .unwrap()
    };
    let dispatch = submit("plain-turn", "refactor the parser")
        .remote_dispatch
        .unwrap();
    runtime
        .owned
        .settle_remote_dispatch_if_current(&dispatch, Some("worker-run-1"))
        .unwrap();
    let session = runtime
        .owned
        .session_store
        .get_session(&session_id)
        .unwrap();
    let id = runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &agent_id)
        .unwrap()
        .id()
        .to_string();
    let id = id.as_str();
    assert!(!needs(id), "empty catalog and no skill syntax");
    let config = runtime.owned.config_projection.snapshot();
    let storage = SkillCatalogStorage::new(
        config.user_config_path.parent().unwrap(),
        runtime.owned.durable_state_store.clone(),
    )
    .unwrap();
    let service = SkillCatalogService::new(storage);
    let mutation = SkillCatalogMutation {
        scope: SkillScope::Chariox,
        expected_revision: 0,
        actions: vec![SkillCatalogAction::Install {
            source: SkillInstallSource::Content {
                skill_md: "---\nname: review\ndescription: Review safely\n---\nReview source.\n"
                    .into(),
            },
            collision: SkillCollisionAction::Reject,
            package_id: None,
        }],
    };
    let prepared = service.prepare(&mutation).unwrap();
    service.publish_authorized(&mutation, &prepared).unwrap();
    assert!(needs(id), "an effective catalog skill needs native dedupe");
}

// MP-08 / MP-10 / MP-11: an ordinary leased prompt from a second named kernel sharing
// the home must not touch the catalog it does not own.
#[tokio::test]
async fn second_named_kernel_leased_turn_skips_inventory_without_reading_the_catalog() {
    let root = std::env::temp_dir().join(format!("skillsk-second-{:032x}", rand::random::<u128>()));
    std::fs::create_dir(&root).unwrap();
    let config = |name: &str| {
        let mut config = crate::DaemonConfig::for_tests();
        config.user_config_path = root.join("home/config.toml");
        config.user_config.state.path =
            Some(root.join(name).join("state.db").display().to_string());
        config
    };
    let owner =
        crate::durable_state::DurableKernelStateStore::open_kernel_owned(&config("owner")).unwrap();
    assert!(owner.owns_skill_catalog());
    let (runtime, session_id, agent_id, attachment, _binding) =
        super::tests::mp11_fixture(config("other")).await;
    assert!(!runtime.owned.durable_state_store.owns_skill_catalog());
    let submission = runtime
        .owned
        .submit_remote_prepared_prompt(&crate::app::KernelPreparedPromptSubmission {
            session_id: session_id.clone(),
            prompt: crate::session::PromptQueueItem::new(
                "ordinary-turn",
                &attachment,
                &agent_id,
                "refactor the parser",
                crate::session::PromptStatus::Queued,
            ),
            force_queue: false,
            refresh_projection: true,
        })
        .unwrap()
        .unwrap();
    runtime
        .owned
        .settle_remote_dispatch_if_current(
            &submission.remote_dispatch.unwrap(),
            Some("worker-run-1"),
        )
        .unwrap();
    let session = runtime
        .owned
        .session_store
        .get_session(&session_id)
        .unwrap();
    let id = runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &agent_id)
        .unwrap()
        .id()
        .to_string();
    let needs = super::super::skills_dispatch::remote_turn_needs_native_inventory(
        &runtime,
        &session_id,
        &agent_id,
        &id,
    );
    drop(owner);
    let _ = std::fs::remove_dir_all(&root);
    assert!(!needs.unwrap());
}

// MP-08 / MP-10 / MP-11: the same kernel's ordinary local structured turn carries no skill
// delivery and is not refused for lack of the catalog it does not own.
#[tokio::test]
async fn second_named_kernel_ordinary_local_turn_is_not_refused_by_the_skills_catalog() {
    let root = std::env::temp_dir().join(format!("skillsk-second-{:032x}", rand::random::<u128>()));
    std::fs::create_dir(&root).unwrap();
    let config = |name: &str| {
        let mut config = crate::DaemonConfig::for_tests();
        config.user_config_path = root.join("home/config.toml");
        config.user_config.state.path =
            Some(root.join(name).join("state.db").display().to_string());
        config
    };
    let owner =
        crate::durable_state::DurableKernelStateStore::open_kernel_owned(&config("owner")).unwrap();
    assert!(owner.owns_skill_catalog());
    let (runtime, session_id, agent_id, attachment, _binding) =
        super::tests::mp11_fixture(config("other")).await;
    assert!(!runtime.owned.durable_state_store.owns_skill_catalog());
    let submission = runtime
        .owned
        .submit_remote_prepared_prompt(&crate::app::KernelPreparedPromptSubmission {
            session_id: session_id.clone(),
            prompt: crate::session::PromptQueueItem::new(
                "ordinary-turn",
                &attachment,
                &agent_id,
                "refactor the parser",
                crate::session::PromptStatus::Queued,
            ),
            force_queue: false,
            refresh_projection: true,
        })
        .unwrap()
        .unwrap();
    runtime
        .owned
        .settle_remote_dispatch_if_current(
            &submission.remote_dispatch.unwrap(),
            Some("worker-run-1"),
        )
        .unwrap();
    let session = runtime
        .owned
        .session_store
        .get_session(&session_id)
        .unwrap();
    let prompt_id = runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &agent_id)
        .unwrap()
        .id()
        .to_string();
    let request = crate::provider::LaunchProviderRequest::new(
        &session_id,
        "dev-stub",
        "claude-code",
        "default",
        "sonnet",
    )
    .with_agent_id(&agent_id);
    let run = crate::provider::RuntimeProviderRun::new(
        "local-run-1",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "second-kernel-local".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    let envelope = crate::prompt_assembly::PromptEnvelope::new(
        "refactor the parser",
        "",
        Vec::new(),
        crate::prompt_assembly::PromptManifest::default(),
    );
    let prepared = runtime.owned.provider_store.prepare_skill_prompt(
        &session_id,
        &agent_id,
        &prompt_id,
        &prompt_id,
        &run,
        envelope,
    );
    drop(owner);
    let _ = std::fs::remove_dir_all(&root);
    assert!(prepared.unwrap().skill_delivery.is_none());
}

// MP-08 / MP-10: an explicitly loaded skill reaches a leased worker once per worker
// provider run; a new worker run re-attaches it.
#[tokio::test]
async fn leased_follow_up_turn_does_not_repeat_a_load_the_worker_run_holds() {
    use crate::skill::service::SkillCatalogService;
    let (runtime, session_id, agent_id, attachment, binding) =
        super::tests::mp11_fixture(crate::DaemonConfig::for_tests()).await;
    let config = runtime.owned.config_projection.snapshot();
    let service = SkillCatalogService::new(
        SkillCatalogStorage::new(
            config.user_config_path.parent().unwrap(),
            runtime.owned.durable_state_store.clone(),
        )
        .unwrap(),
    );
    let mutation = SkillCatalogMutation {
        scope: SkillScope::Chariox, expected_revision: 0,
        actions: vec![SkillCatalogAction::Install {
            source: SkillInstallSource::Content { skill_md: "---\nname: review\ndescription: Review safely\ndisable-model-invocation: true\n---\nReview source.\n".into() },
            collision: SkillCollisionAction::Reject, package_id: None,
        }],
    };
    let prepared = service.prepare(&mutation).unwrap();
    let catalog = service.publish_authorized(&mutation, &prepared).unwrap();
    let skill = &catalog.catalogs[0].skills[0];
    let selection = SkillSelection::Chariox {
        skill_id: skill.id.clone(),
        name: skill.name.clone(),
        catalog_revision: catalog.revision,
    };
    let project_run = |worker_run: &str| {
        let projected =
            crate::provider::projected_leased_provider_run_id(&binding.leased_agent_id, worker_run);
        let mut run = crate::provider::RuntimeProviderRun::from_control_capability_inference(
            worker_run,
            "worker-session".into(),
            Some(binding.leased_agent_id.clone()),
            "dev-stub".into(),
        );
        run.mark_running();
        let run = run.projected_for_home_agent_with_id(&projected, &session_id, &agent_id);
        runtime
            .owned
            .provider_run_projection
            .update_remote_snapshot(run);
    };
    let turn = |id: &str, text: &str, selections: Vec<SkillSelection>| {
        let submission = runtime
            .owned
            .submit_remote_prepared_prompt(&crate::app::KernelPreparedPromptSubmission {
                session_id: session_id.clone(),
                prompt: crate::session::PromptQueueItem::new(
                    id,
                    &attachment,
                    &agent_id,
                    text,
                    crate::session::PromptStatus::Queued,
                )
                .with_skill_selections(selections)
                .with_user_skill_syntax(true),
                force_queue: false,
                refresh_projection: true,
            })
            .unwrap()
            .unwrap();
        runtime
            .owned
            .settle_remote_dispatch_if_current(
                &submission.remote_dispatch.unwrap(),
                Some("worker-run-1"),
            )
            .unwrap();
        let session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .unwrap();
        runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &agent_id)
            .unwrap()
    };
    let remote = |prompt_id: &str| {
        super::super::skills_dispatch::prepare_remote_turn(
            &runtime,
            &session_id,
            &agent_id,
            prompt_id,
            &binding.leased_agent_id,
            &[],
        )
        .unwrap()
    };
    project_run("worker-run-1");
    let first = turn("load-turn", "$review inspect source", vec![selection]);
    assert_eq!(remote(first.id()).unwrap().packages.len(), 1);
    let session = runtime
        .owned
        .session_store
        .get_session(&session_id)
        .unwrap();
    let context = RemoteWorkspaceLiveSyncContext {
        home_kernel_id: config.daemon_id.clone(),
        home_session_id: session_id.clone(),
        home_agent_id: agent_id.clone(),
        home_prompt_id: Some(first.id().into()),
        leased_agent_id: binding.leased_agent_id.clone(),
        worker_kernel_id: binding.worker_kernel_id.clone(),
        worker_machine_id: binding.worker_machine_id.clone(),
        worker_provider_run_id: "worker-run-1".into(),
        worker_worktree_path: session.worktree_id().into(),
        worker_workspace_identity: crate::io::WorkspaceIdentity::local("fixture"),
    };
    acknowledge(&runtime, &context, serde_json::json!({"delivery":"prompt"}))
        .await
        .unwrap();
    runtime
        .owned
        .prompt_state_owner
        .complete_active_prompt_only(&session, &agent_id);
    let second = turn("follow-up", "continue reviewing", vec![]);
    assert!(
        remote(second.id()).is_none_or(|turn| turn.packages.is_empty()),
        "the worker run already holds the load"
    );
    runtime
        .owned
        .prompt_state_owner
        .complete_active_prompt_only(&session, &agent_id);
    project_run("worker-run-2");
    let third = turn("after-restart", "continue reviewing", vec![]);
    assert_eq!(
        remote(third.id()).unwrap().packages.len(),
        1,
        "a new worker run lost the load"
    );
}
