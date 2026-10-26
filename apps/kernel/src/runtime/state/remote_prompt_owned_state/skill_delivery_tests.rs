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
    fixture_with_dispatch(true).await
}

/// `settled` is false while the worker has not yet acknowledged the submit.
async fn fixture_with_dispatch(
    settled: bool,
) -> (
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
    let mut prompt = runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &agent_id)
        .unwrap();
    if !settled {
        // A promoted queued prompt stays Dispatching until its worker settles the submit.
        let mut dispatching = prompt.clone();
        dispatching.set_status(crate::session::PromptStatus::Dispatching);
        assert!(runtime
            .owned
            .prompt_state_owner
            .replace_active_prompt_if_matches(&session, &agent_id, &prompt, dispatching.clone()));
        prompt = dispatching;
    }
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
        native_tool_loads: Default::default(),
        compaction_revision: None,
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

pub(super) async fn acknowledge(
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
        &super::super::skills_dispatch::ReportedWorkerRun::Unknown,
    )
    .unwrap()
    .unwrap();
    assert!(remote.set.manifest.skills.is_empty());
    assert!(remote.set.packages.is_empty());
    // A skill the worker's harness provides is neither transferred nor named for typing.
    assert!(remote.set.names.is_none());
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

/// One leased turn that selects `review` on a worker harness `provider`: the Chariox
/// copy (deduped against the equal native one) or only the native entry.
async fn leased_native_selection_turn(
    provider: &str,
    chariox_copy: bool,
) -> (
    KernelRuntimeState,
    RemoteWorkspaceLiveSyncContext,
    SkillCatalogStorage,
    SkillTurnAuthority,
    String,
) {
    use crate::skill::service::SkillCatalogService;
    let (runtime, session_id, agent_id, attachment, binding) =
        super::tests::mp11_fixture(crate::DaemonConfig::for_tests()).await;
    let config = runtime.owned.config_projection.snapshot();
    let storage = SkillCatalogStorage::new(
        config.user_config_path.parent().unwrap(),
        runtime.owned.durable_state_store.clone(),
    )
    .unwrap();
    let digest = "d".repeat(64);
    let native_selection = SkillSelection::Native {
        entry_id: "native-review".into(),
        name: "review".into(),
        provider: provider.into(),
        account_profile: "account".into(),
        execution_id: "worker-run-1".into(),
        inventory_revision: "native-revision".into(),
    };
    let (selection, digest, expected_id) = if chariox_copy {
        let service = SkillCatalogService::new(storage.clone());
        let mutation = SkillCatalogMutation {
            scope: SkillScope::Chariox,
            expected_revision: 0,
            actions: vec![SkillCatalogAction::Install {
                source: SkillInstallSource::Content {
                    skill_md: "---\nname: review\ndescription: Review safely\ndisable-model-invocation: true\n---\nReview source.\n".into(),
                },
                collision: SkillCollisionAction::Reject,
                package_id: None,
            }],
        };
        let prepared = service.prepare(&mutation).unwrap();
        let catalog = service.publish_authorized(&mutation, &prepared).unwrap();
        let skill = &catalog.catalogs[0].skills[0];
        (
            SkillSelection::Chariox {
                skill_id: skill.id.clone(),
                name: skill.name.clone(),
                catalog_revision: catalog.revision,
            },
            skill.digest.clone(),
            skill.id.clone(),
        )
    } else {
        (
            native_selection.clone(),
            digest,
            "native-review".to_string(),
        )
    };
    let native = NativeSkillEntry {
        selection: native_selection,
        description: "Review safely".into(),
        digest: Some(digest),
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
                "native-selection-turn",
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
    let prompt = runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &agent_id)
        .unwrap();
    super::super::skills_dispatch::prepare_remote_turn(
        &runtime,
        &session_id,
        &agent_id,
        prompt.id(),
        &binding.leased_agent_id,
        &[native],
        &super::super::skills_dispatch::ReportedWorkerRun::Unknown,
    )
    .unwrap()
    .unwrap();
    let prompt = runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &agent_id)
        .unwrap();
    let authority = prompt.skill_turn_authority().unwrap().clone();
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
    (runtime, context, storage, authority, expected_id)
}

// A native-only `From Codex` selection has no Chariox row: the home still reserves a
// receipt for it, and the worker's real prompt delivery settles it as Loaded.
#[tokio::test]
async fn leased_native_only_selection_is_recorded_loaded_on_the_home() {
    let (runtime, context, storage, authority, skill_id) =
        leased_native_selection_turn("codex", false).await;
    assert_eq!(authority.pending_loads.len(), 1);
    assert_eq!(authority.pending_loads[0].skill_id, skill_id);
    assert_eq!(
        authority.pending_loads[0].reason,
        SkillLoadReason::ConfirmedNative
    );
    assert!(authority.prompt_delivery_skill_ids.contains(&skill_id));
    assert!(storage.loads(&context.home_session_id).unwrap().is_empty());
    acknowledge(&runtime, &context, serde_json::json!({"delivery":"prompt"}))
        .await
        .unwrap();
    let loads = storage.loads(&context.home_session_id).unwrap();
    assert_eq!(loads.len(), 1);
    assert_eq!(loads[0].skill_id, skill_id);
}

// OpenCode loads a native-only selection with its own `skill` tool: the worker reports
// the completed tool by name and the home settles the load that name maps to.
#[tokio::test]
async fn leased_opencode_native_only_selection_settles_on_the_native_tool_receipt() {
    let (runtime, context, storage, authority, skill_id) =
        leased_native_selection_turn("opencode", false).await;
    assert_eq!(
        authority.native_tool_loads.get("review"),
        Some(&skill_id),
        "home maps the tool name to its pending load"
    );
    acknowledge(&runtime, &context, serde_json::json!({"delivery":"prompt"}))
        .await
        .unwrap();
    acknowledge(
        &runtime,
        &context,
        serde_json::json!({"delivery":"native_tool","name":"unrelated"}),
    )
    .await
    .unwrap();
    assert!(storage.loads(&context.home_session_id).unwrap().is_empty());
    acknowledge(
        &runtime,
        &context,
        serde_json::json!({"delivery":"native_tool","name":"review"}),
    )
    .await
    .unwrap();
    let loads = storage.loads(&context.home_session_id).unwrap();
    assert_eq!(loads.len(), 1);
    assert_eq!(loads[0].skill_id, skill_id);
}

/// What a leased worker holds for its turn: the prompt carries the home's lease context.
/// Returns the runtime, ids and the worker run (marked leased) for receipt tests.
async fn worker_leased_turn(
    config: crate::DaemonConfig,
    provider: &str,
) -> (
    KernelRuntimeState,
    String,
    String,
    String,
    crate::provider::RuntimeProviderRun,
) {
    let (runtime, session_id, agent_id, attachment, binding) =
        super::tests::mp11_fixture(config).await;
    let daemon_id = runtime.owned.config_projection.snapshot().daemon_id.clone();
    let session = runtime
        .owned
        .session_store
        .get_session(&session_id)
        .unwrap();
    let context = RemoteWorkspaceLiveSyncContext {
        home_kernel_id: daemon_id,
        home_session_id: session_id.clone(),
        home_agent_id: agent_id.clone(),
        home_prompt_id: Some("home-prompt".into()),
        leased_agent_id: binding.leased_agent_id,
        worker_kernel_id: binding.worker_kernel_id,
        worker_machine_id: binding.worker_machine_id,
        worker_provider_run_id: "worker-run-1".into(),
        worker_worktree_path: session.worktree_id().into(),
        worker_workspace_identity: crate::io::WorkspaceIdentity::local("fixture"),
    };
    let submission = runtime
        .owned
        .submit_remote_prepared_prompt(&crate::app::KernelPreparedPromptSubmission {
            session_id: session_id.clone(),
            prompt: crate::session::PromptQueueItem::new(
                "worker-turn",
                &attachment,
                &agent_id,
                "inspect source",
                crate::session::PromptStatus::Queued,
            )
            .with_leased_skills(Default::default(), Some(context)),
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
    let prompt_id = runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &agent_id)
        .unwrap()
        .id()
        .to_string();
    let request = crate::provider::LaunchProviderRequest::new(
        &session_id,
        provider,
        provider,
        "default",
        "model",
    )
    .with_agent_id(&agent_id);
    let mut run = crate::provider::RuntimeProviderRun::new(
        "worker-run-1",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "leased-worker".into(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: Default::default(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("test-worker".into()),
        },
    );
    run.mark_running();
    runtime
        .owned
        .provider_run_projection
        .mark_leased_provider_run(run.id());
    (runtime, session_id, agent_id, prompt_id, run)
}

// The drill's first failing seam: on the worker, OpenCode's completed `skill` call must
// reach the home through the lease even though the worker's run is a leased run.
#[tokio::test]
async fn worker_forwards_opencode_native_tool_receipt_for_its_leased_run() {
    let (runtime, _session_id, _agent_id, _prompt_id, run) =
        worker_leased_turn(crate::DaemonConfig::for_tests(), "opencode").await;
    let batch = crate::provider::ProviderPromptSignalBatch {
        chunks: vec![crate::provider::ProviderPromptChunk {
            kind: crate::terminal::TerminalOutputKind::ProviderTool,
            merge_key: None,
            bytes: br#"{"tool":"skill","status":"completed","output":"loaded","input":{"name":"review"}}"#.to_vec(),
        }],
        ..Default::default()
    };
    let bridge = super::super::skills_dispatch::bridge(&runtime);
    // No home peer exists in this fixture: reaching for it is the observable proof
    // the receipt was forwarded instead of silently dropped.
    let error = bridge
        .acknowledge_tools(&run, &batch)
        .unwrap_err()
        .to_string();
    assert!(!error.is_empty());
}

// A Chariox skill deduped against an equal OpenCode package is loaded by OpenCode's
// own tool; its completed call settles the pending ProvidedBy load on the home.
#[tokio::test]
async fn leased_opencode_deduped_selection_settles_on_the_native_tool_receipt() {
    let (runtime, context, storage, authority, skill_id) =
        leased_native_selection_turn("opencode", true).await;
    assert_eq!(authority.native_tool_loads.get("review"), Some(&skill_id));
    assert!(storage.loads(&context.home_session_id).unwrap().is_empty());
    acknowledge(
        &runtime,
        &context,
        serde_json::json!({"delivery":"native_tool","name":"review"}),
    )
    .await
    .unwrap();
    let loads = storage.loads(&context.home_session_id).unwrap();
    assert_eq!(loads.len(), 1);
    assert_eq!(loads[0].skill_id, skill_id);
}

// A Claude copy that cannot acknowledge its own load cannot own the turn's one
// body: the home transfers its equal Chariox copy and delivers that instead.
#[tokio::test]
async fn leased_claude_native_copy_without_ack_is_delivered_by_chariox() {
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
    let catalog = service.publish_authorized(&mutation, &prepared).unwrap();
    let skill = catalog.catalogs[0].skills[0].clone();
    let native = NativeSkillEntry {
        selection: SkillSelection::Native {
            entry_id: "native-review".into(),
            name: skill.name.clone(),
            provider: "claude".into(),
            account_profile: "account".into(),
            execution_id: "worker-run-1".into(),
            inventory_revision: "native-revision".into(),
        },
        description: skill.description.clone(),
        digest: Some(skill.digest.clone()),
        exposure: NativeSkillExposure::Confirmed,
        default_state: SkillState::Auto,
        complete_manifest: true,
        supports_load_ack: false,
        discovery_error: None,
    };
    let submission = runtime
        .owned
        .submit_remote_prepared_prompt(&crate::app::KernelPreparedPromptSubmission {
            session_id: session_id.clone(),
            prompt: crate::session::PromptQueueItem::new(
                "claude-dedupe-turn",
                &attachment,
                &agent_id,
                "$review inspect source",
                crate::session::PromptStatus::Queued,
            )
            .with_skill_selections(vec![SkillSelection::Chariox {
                skill_id: skill.id.clone(),
                name: skill.name.clone(),
                catalog_revision: catalog.revision,
            }])
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
        &[native],
        &super::super::skills_dispatch::ReportedWorkerRun::Unknown,
    )
    .unwrap()
    .expect("the Chariox copy is delivered");
    assert!(remote.native_selections.is_empty());
    assert_eq!(remote.set.manifest.skills.len(), 1);
    assert_eq!(remote.set.packages.len(), 1);
    let context = remote
        .hidden_context(&[crate::transport::relay_peer::RemoteSkillMaterialization {
            skill_id: skill.id.clone(),
            name: "review".into(),
            version_hash: skill.digest.clone(),
            materialized_root: "/worker/review".into(),
        }])
        .unwrap();
    assert_eq!(context.matches("Review source.").count(), 1, "{context:?}");
    let authority = runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &agent_id)
        .unwrap()
        .skill_turn_authority()
        .cloned()
        .unwrap();
    assert_eq!(authority.pending_loads.len(), 1);
    assert!(authority.prompt_delivery_skill_ids.contains(&skill.id));
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

// The headless worker acknowledges its submit only after the hook delivered the
// body, and reports that receipt while the home turn is still Dispatching.
#[tokio::test]
async fn leased_delivery_receipt_is_admitted_while_the_home_turn_is_dispatching() {
    let (runtime, context, storage, _) = fixture_with_dispatch(false).await;
    let session = runtime
        .owned
        .session_store
        .get_session(&context.home_session_id)
        .unwrap();
    let prompt = runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &context.home_agent_id)
        .unwrap();
    assert_eq!(prompt.status(), crate::session::PromptStatus::Dispatching);
    acknowledge(&runtime, &context, serde_json::json!({"delivery":"prompt"}))
        .await
        .unwrap();
    assert_eq!(storage.loads(&context.home_session_id).unwrap().len(), 1);
    // Every other forwarded tool still needs the running turn, and a receipt
    // still needs the exact prompt.
    let peer = runtime.with_relay_peer_authority(
        crate::runtime::relay_peer_authority::test_peer_authority(&context.worker_kernel_id),
    );
    assert!(peer
        .authorize_forwarded_workspace_context(&context)
        .is_err());
    let mut other = context.clone();
    other.home_prompt_id = Some("another-prompt".into());
    assert!(peer
        .authorize_forwarded_delivery_receipt_context(&other)
        .is_err());
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
    second_named_kernel_leased_turn_skips_inventory("refactor the parser", false).await;
}

// Unresolved dollar prose is an ordinary prompt: no selection resolved it.
#[tokio::test]
async fn second_named_kernel_leased_dollar_prose_skips_inventory() {
    second_named_kernel_leased_turn_skips_inventory("why is $total null", true).await;
}

async fn second_named_kernel_leased_turn_skips_inventory(text: &str, user_skill_syntax: bool) {
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
                text,
                crate::session::PromptStatus::Queued,
            )
            .with_user_skill_syntax(user_skill_syntax),
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

// A second named worker sharing the home directory owns no catalog, yet it delivers the
// authenticated home's instructions: its delivery receipt still has to reach the home.
#[tokio::test]
async fn second_named_worker_forwards_prompt_delivery_receipts_to_the_home() {
    let root = std::env::temp_dir().join(format!("skillsk-worker-{:032x}", rand::random::<u128>()));
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
    let (runtime, session_id, agent_id, prompt_id, run) =
        worker_leased_turn(config("other"), "codex").await;
    assert!(!runtime.owned.durable_state_store.owns_skill_catalog());
    let bridge = super::super::skills_dispatch::bridge(&runtime);
    // No home peer exists in this fixture: reaching for it is the observable proof the
    // receipt was routed to the home instead of dropped for lack of a local catalog.
    let result = bridge.acknowledge_prompt(&run, &session_id, &agent_id, &prompt_id);
    drop(owner);
    let _ = std::fs::remove_dir_all(&root);
    assert!(result.is_err(), "the receipt never left the worker");
}

// MP-08 / MP-10 / MP-11: the same kernel's ordinary local structured turn carries no skill
// delivery and is not refused for lack of the catalog it does not own.
#[tokio::test]
async fn second_named_kernel_ordinary_local_turn_is_not_refused_by_the_skills_catalog() {
    second_named_kernel_local_turn_is_not_refused("refactor the parser", false).await;
}

#[tokio::test]
async fn second_named_kernel_local_dollar_prose_is_not_refused_by_the_skills_catalog() {
    second_named_kernel_local_turn_is_not_refused("why is $total null", true).await;
}

async fn second_named_kernel_local_turn_is_not_refused(text: &str, user_skill_syntax: bool) {
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
                text,
                crate::session::PromptStatus::Queued,
            )
            .with_user_skill_syntax(user_skill_syntax),
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
        text,
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
// provider run. The home learns the run only when a turn settles, so a worker run it
// saw settle is the run it expects; a run it sees change re-attaches the load.
#[tokio::test]
async fn leased_follow_up_turn_does_not_repeat_a_load_the_worker_run_holds() {
    leased_load_is_delivered_once_per_worker_run(Relaunch::Settled).await;
}

// The home projection of a different worker run than the last settled one is a run
// the home cannot vouch for: the load is delivered again.
#[tokio::test]
async fn leased_turn_re_attaches_a_load_when_the_projected_worker_run_differs() {
    leased_load_is_delivered_once_per_worker_run(Relaunch::Projected).await;
}

// A worker relaunch the home never saw settle or project: the worker names the run that
// takes the turn when the home reads its inventory, and the load rides that turn.
#[tokio::test]
async fn leased_turn_re_attaches_a_load_when_the_worker_reports_a_new_run_at_turn_start() {
    leased_load_is_delivered_once_per_worker_run(Relaunch::ReportedAtTurnStart).await;
}

// The worker's harness compacted its conversation inside the run the home still sees:
// the load rides the next turn once, and a replayed report does not repeat it.
#[tokio::test]
async fn leased_turn_re_attaches_a_load_after_a_worker_compaction_exactly_once() {
    leased_load_is_delivered_once_per_worker_run(Relaunch::Compacted).await;
}

enum Relaunch {
    Compacted,
    Settled,
    Projected,
    ReportedAtTurnStart,
}

async fn leased_load_is_delivered_once_per_worker_run(relaunch: Relaunch) {
    use super::super::skills_dispatch::ReportedWorkerRun;
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
    let session = || {
        runtime
            .owned
            .session_store
            .get_session(&session_id)
            .unwrap()
    };
    // The real order: the home prepares the turn, dispatches it, and learns the worker
    // run from the settlement. No test code touches the binding or the projection.
    let submit = |id: &str, text: &str, selections: Vec<SkillSelection>| {
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
        submission.remote_dispatch.unwrap()
    };
    // Completion clears the binding's active worker run, as in the real flow.
    let complete = |worker_run: &str| {
        runtime
            .owned
            .complete_remote_prompt_owner(&session_id, &agent_id, worker_run, None)
            .unwrap();
    };
    let settle = |dispatch: &crate::app::KernelRemotePromptDispatch, worker_run: &str| {
        runtime
            .owned
            .settle_remote_dispatch_if_current(dispatch, Some(worker_run))
            .unwrap();
        complete(worker_run);
    };
    let remote_reported = |prompt_id: &str, worker_run: ReportedWorkerRun| {
        super::super::skills_dispatch::prepare_remote_turn(
            &runtime,
            &session_id,
            &agent_id,
            prompt_id,
            &binding.leased_agent_id,
            &[],
            &worker_run,
        )
        .unwrap()
    };
    let remote = |prompt_id: &str| remote_reported(prompt_id, ReportedWorkerRun::Unknown);
    // The Manual skill is transferred with every turn; only its body rides a load.
    let roots = |turn: &super::super::skills_dispatch::RemoteSkillTurn| {
        let skill = &turn.set.manifest.skills[0];
        turn.hidden_context(&[crate::transport::relay_peer::RemoteSkillMaterialization {
            skill_id: skill.skill_id.clone(),
            name: "review".into(),
            version_hash: skill.digest.clone(),
            materialized_root: "/worker/review".into(),
        }])
        .unwrap()
    };
    let first = submit("load-turn", "$review inspect source", vec![selection]);
    let first_turn = remote(&first.prompt_id).unwrap();
    assert_eq!(first_turn.set.packages.len(), 1);
    assert!(roots(&first_turn).contains("Review source."));
    runtime
        .owned
        .settle_remote_dispatch_if_current(&first, Some("worker-run-1"))
        .unwrap();
    let context = RemoteWorkspaceLiveSyncContext {
        home_kernel_id: config.daemon_id.clone(),
        home_session_id: session_id.clone(),
        home_agent_id: agent_id.clone(),
        home_prompt_id: Some(first.prompt_id.clone()),
        leased_agent_id: binding.leased_agent_id.clone(),
        worker_kernel_id: binding.worker_kernel_id.clone(),
        worker_machine_id: binding.worker_machine_id.clone(),
        worker_provider_run_id: "worker-run-1".into(),
        worker_worktree_path: session().worktree_id().into(),
        worker_workspace_identity: crate::io::WorkspaceIdentity::local("fixture"),
    };
    acknowledge(&runtime, &context, serde_json::json!({"delivery":"prompt"}))
        .await
        .unwrap();
    complete("worker-run-1");
    let second = submit("follow-up", "continue reviewing", vec![]);
    assert!(
        remote_reported(
            &second.prompt_id,
            ReportedWorkerRun::Live("worker-run-1".into())
        )
        .is_none_or(|turn| !roots(&turn).contains("Review source.")),
        "the worker run already holds the load"
    );
    if matches!(relaunch, Relaunch::Compacted) {
        settle(&second, "worker-run-1");
        let compacted = |ids: &[&str]| {
            super::super::skills_dispatch::record_worker_compactions(
                &runtime,
                &session_id,
                &agent_id,
                &binding.leased_agent_id,
                &ids.iter().map(|id| id.to_string()).collect::<Vec<_>>(),
            )
            .unwrap()
        };
        let live = || ReportedWorkerRun::Live("worker-run-1".into());
        compacted(&["compact-1"]);
        let third = submit("after-compaction", "continue reviewing", vec![]);
        assert!(
            roots(&remote_reported(&third.prompt_id, live()).unwrap()).contains("Review source."),
            "a compacted worker conversation lost the load"
        );
        runtime
            .owned
            .settle_remote_dispatch_if_current(&third, Some("worker-run-1"))
            .unwrap();
        let third_context = RemoteWorkspaceLiveSyncContext {
            home_prompt_id: Some(third.prompt_id.clone()),
            ..context.clone()
        };
        acknowledge(
            &runtime,
            &third_context,
            serde_json::json!({"delivery":"prompt"}),
        )
        .await
        .unwrap();
        complete("worker-run-1");
        compacted(&["compact-1"]);
        let fourth = submit("after-recovery", "continue reviewing", vec![]);
        assert!(
            remote_reported(&fourth.prompt_id, live())
                .is_none_or(|turn| !roots(&turn).contains("Review source.")),
            "one compaction reloads once"
        );
        return;
    }
    // The worker relaunched: the home sees it as the next settlement or projection, or
    // only from the worker's own report when the next turn starts.
    let reported = match relaunch {
        Relaunch::Projected => {
            settle(&second, "worker-run-1");
            project_run("worker-run-2");
            ReportedWorkerRun::Unknown
        }
        Relaunch::Compacted => unreachable!("returned above"),
        Relaunch::Settled => {
            settle(&second, "worker-run-2");
            ReportedWorkerRun::Unknown
        }
        Relaunch::ReportedAtTurnStart => {
            settle(&second, "worker-run-1");
            ReportedWorkerRun::Live("worker-run-2".into())
        }
    };
    let third = submit("after-restart", "continue reviewing", vec![]);
    if matches!(relaunch, Relaunch::ReportedAtTurnStart) {
        assert!(
            remote(&third.prompt_id).is_none_or(|turn| !roots(&turn).contains("Review source.")),
            "without the worker's report the home assumes the held run"
        );
        assert!(
            roots(&remote_reported(&third.prompt_id, ReportedWorkerRun::NoRun).unwrap())
                .contains("Review source."),
            "a lease without a provider run starts a new one"
        );
    }
    assert!(
        roots(&remote_reported(&third.prompt_id, reported).unwrap()).contains("Review source."),
        "a new worker run lost the load"
    );
}

// A compaction loses every earlier load, so one receipt for one reloaded skill must not
// make the other look held on the next turn.
#[tokio::test]
async fn leased_compaction_keeps_a_skill_pending_until_its_own_reload_is_acknowledged() {
    partial_compaction_recovery(false).await;
    partial_compaction_recovery(true).await;
}

async fn partial_compaction_recovery(other_session_is_newer: bool) {
    use super::super::skills_dispatch::ReportedWorkerRun;
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
    let install = |name: &str, body: &str| {
        SkillCatalogAction::Install {
        source: SkillInstallSource::Content { skill_md: format!("---\nname: {name}\ndescription: {name} safely\ndisable-model-invocation: true\n---\n{body}\n") },
        collision: SkillCollisionAction::Reject, package_id: None,
    }
    };
    let mutation = SkillCatalogMutation {
        scope: SkillScope::Chariox,
        expected_revision: 0,
        actions: vec![
            install("review", "Review source."),
            install("audit", "Audit source."),
        ],
    };
    let prepared = service.prepare(&mutation).unwrap();
    let catalog = service.publish_authorized(&mutation, &prepared).unwrap();
    let skills = &catalog.catalogs[0].skills;
    let find = |name: &str| {
        skills
            .iter()
            .find(|skill| skill.name == name)
            .unwrap()
            .clone()
    };
    let (review, audit) = (find("review"), find("audit"));
    let select = |skill: &SkillRecord| SkillSelection::Chariox {
        skill_id: skill.id.clone(),
        name: skill.name.clone(),
        catalog_revision: catalog.revision,
    };
    let session = || {
        runtime
            .owned
            .session_store
            .get_session(&session_id)
            .unwrap()
    };
    let submit = |id: &str, text: &str, selections: Vec<SkillSelection>| {
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
                .with_skill_selections(selections)
                .with_user_skill_syntax(true),
                force_queue: false,
                refresh_projection: true,
            })
            .unwrap()
            .unwrap()
            .remote_dispatch
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
            &ReportedWorkerRun::Live("worker-run-1".into()),
        )
        .unwrap()
    };
    let bodies = |turn: Option<super::super::skills_dispatch::RemoteSkillTurn>| {
        turn.map(|turn| {
            turn.hidden_context(
                &turn
                    .set
                    .manifest
                    .skills
                    .iter()
                    .map(|entry| {
                        let name = skills
                            .iter()
                            .find(|skill| skill.id == entry.skill_id)
                            .unwrap()
                            .name
                            .clone();
                        crate::transport::relay_peer::RemoteSkillMaterialization {
                            skill_id: entry.skill_id.clone(),
                            materialized_root: format!("/worker/{name}"),
                            name,
                            version_hash: entry.digest.clone(),
                        }
                    })
                    .collect::<Vec<_>>(),
            )
            .unwrap()
        })
        .unwrap_or_default()
    };
    let context_for = |prompt_id: &str| RemoteWorkspaceLiveSyncContext {
        home_kernel_id: config.daemon_id.clone(),
        home_session_id: session_id.clone(),
        home_agent_id: agent_id.clone(),
        home_prompt_id: Some(prompt_id.into()),
        leased_agent_id: binding.leased_agent_id.clone(),
        worker_kernel_id: binding.worker_kernel_id.clone(),
        worker_machine_id: binding.worker_machine_id.clone(),
        worker_provider_run_id: "worker-run-1".into(),
        worker_worktree_path: session().worktree_id().into(),
        worker_workspace_identity: crate::io::WorkspaceIdentity::local("fixture"),
    };
    let settle = |dispatch: &crate::app::KernelRemotePromptDispatch| {
        runtime
            .owned
            .settle_remote_dispatch_if_current(dispatch, Some("worker-run-1"))
            .unwrap();
    };
    let complete = || {
        runtime
            .owned
            .complete_remote_prompt_owner(&session_id, &agent_id, "worker-run-1", None)
            .unwrap();
    };
    let first = submit(
        "load-both",
        "$review $audit inspect",
        vec![select(&review), select(&audit)],
    );
    let loaded = bodies(remote(&first.prompt_id));
    assert!(loaded.contains("Review source.") && loaded.contains("Audit source."));
    settle(&first);
    acknowledge(
        &runtime,
        &context_for(&first.prompt_id),
        serde_json::json!({"delivery":"prompt"}),
    )
    .await
    .unwrap();
    complete();
    let other_boundary = || {
        runtime
            .owned
            .operational_history_store
            .record_agent_compaction_boundary(
                &session_id,
                &agent_id,
                "claude",
                "haiku",
                "other-account",
                Some("other-native"),
            )
            .unwrap()
    };
    let older = (!other_session_is_newer).then(other_boundary);
    super::super::skills_dispatch::record_worker_compactions(
        &runtime,
        &session_id,
        &agent_id,
        &binding.leased_agent_id,
        &["compact-1".to_string()],
    )
    .unwrap();
    let second = submit("after-compaction", "continue", vec![]);
    let reloaded = bodies(remote(&second.prompt_id));
    assert!(
        reloaded.contains("Review source.") && reloaded.contains("Audit source."),
        "a compaction lost both loads"
    );
    settle(&second);
    // The worker's harness took only one of the two reloads.
    acknowledge(
        &runtime,
        &context_for(&second.prompt_id),
        serde_json::json!({"delivery":"tool","skill_id":review.id,"digest":review.digest}),
    )
    .await
    .unwrap();
    complete();
    let other = older.unwrap_or_else(other_boundary);
    runtime
        .owned
        .operational_history_store
        .deliver_agent_compaction_boundary(&session_id, &agent_id, other)
        .unwrap();
    let third = submit("next-turn", "continue", vec![]);
    assert!(
        bodies(remote(&third.prompt_id)).contains("Audit source."),
        "one receipt cleared the compaction for a skill that was never reloaded"
    );
    settle(&third);
    // Only the skill still missing is acknowledged; the first one's receipt from the
    // previous turn must still count in this compaction epoch.
    acknowledge(
        &runtime,
        &context_for(&third.prompt_id),
        serde_json::json!({"delivery":"tool","skill_id":audit.id,"digest":audit.digest}),
    )
    .await
    .unwrap();
    complete();
    let fourth = submit("settled", "continue", vec![]);
    let held = bodies(remote(&fourth.prompt_id));
    assert!(
        !held.contains("Audit source.") && !held.contains("Review source."),
        "every skill was reloaded; nothing is pending"
    );
}

pub(super) async fn manual_review_fixture() -> (
    KernelRuntimeState,
    crate::skill::service::SkillCatalogService,
    SkillRecord,
    RemoteWorkspaceLiveSyncContext,
    String,
    String,
) {
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
    let skill = catalog.catalogs[0].skills[0].clone();
    let submission = runtime
        .owned
        .submit_remote_prepared_prompt(&crate::app::KernelPreparedPromptSubmission {
            session_id: session_id.clone(),
            prompt: crate::session::PromptQueueItem::new(
                "plain-turn",
                &attachment,
                &agent_id,
                "continue",
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
    // Text typed on the worker has no dispatched home turn.
    let context = RemoteWorkspaceLiveSyncContext {
        home_kernel_id: config.daemon_id.clone(),
        home_session_id: session_id.clone(),
        home_agent_id: agent_id.clone(),
        home_prompt_id: None,
        leased_agent_id: binding.leased_agent_id,
        worker_kernel_id: binding.worker_kernel_id,
        worker_machine_id: binding.worker_machine_id,
        worker_provider_run_id: "worker-run-1".into(),
        worker_worktree_path: session.worktree_id().into(),
        worker_workspace_identity: crate::io::WorkspaceIdentity::local("fixture"),
    };
    (runtime, service, skill, context, session_id, agent_id)
}

// MP-08 / MP-10: a Manual skill must be on the worker before the user types its
// name there, and the home names it; the turn's own context still never lists it.
#[tokio::test]
async fn leased_turn_transfers_manual_skills_and_names_them_without_advertising_them() {
    let (runtime, _service, skill, _context, session_id, agent_id) = manual_review_fixture().await;
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
    let binding = runtime
        .owned
        .agent_store
        .get_agent(&agent_id)
        .unwrap()
        .remote_execution()
        .cloned()
        .unwrap();
    let turn = super::super::skills_dispatch::prepare_remote_turn(
        &runtime,
        &session_id,
        &agent_id,
        prompt.id(),
        &binding.leased_agent_id,
        &[],
        &super::super::skills_dispatch::ReportedWorkerRun::Unknown,
    )
    .unwrap()
    .expect("a Manual skill is transferred with the turn");
    assert_eq!(turn.set.manifest.skills.len(), 1);
    assert_eq!(turn.set.manifest.skills[0].state, SkillState::Manual);
    assert_eq!(turn.set.packages.len(), 1);
    assert_eq!(
        turn.set.names,
        Some(crate::skill::leased_names::LeasedSkillNames {
            catalog_revision: turn.catalog_revision,
            skills: vec![crate::skill::leased_names::LeasedSkillName {
                skill_id: skill.id.clone(),
                name: "review".into(),
                digest: skill.digest.clone(),
            }],
        })
    );
    let advertised = turn
        .hidden_context(&[crate::transport::relay_peer::RemoteSkillMaterialization {
            skill_id: skill.id,
            name: "review".into(),
            version_hash: skill.digest,
            materialized_root: "/worker/review".into(),
        }])
        .unwrap();
    assert!(
        advertised.is_empty(),
        "a Manual skill was advertised: {advertised:?}"
    );
}

// MP-08 / MP-10: a native composer types `$name` before the home dispatches any turn,
// so the launch itself names and carries every effective skill.
#[tokio::test]
async fn leased_native_launch_carries_the_manual_skill_before_any_turn() {
    let (runtime, _service, skill, _context, session_id, _agent_id) = manual_review_fixture().await;
    let set = super::super::skills_dispatch::prepare_remote_launch_skills(&runtime, &session_id)
        .unwrap()
        .expect("the launch carries the catalog's skills");
    assert_eq!(set.manifest.skills.len(), 1);
    assert_eq!(set.manifest.skills[0].skill_id, skill.id);
    assert_eq!(set.packages.len(), 1);
    assert_eq!(
        set.names.map(|names| names.skills.len()),
        Some(1),
        "the launch left the worker without the name view"
    );
}

// MP-08 / MP-10: when the last effective skill is set Off, the lease must still get a
// view, an empty one, or its composer keeps resolving the skill that was turned off.
#[tokio::test]
async fn leased_native_launch_clears_the_view_when_no_skill_is_effective() {
    let (runtime, service, skill, _context, session_id, _agent_id) = manual_review_fixture().await;
    let off = SkillCatalogMutation {
        scope: SkillScope::Chariox,
        expected_revision: service.storage.snapshot().unwrap().revision,
        actions: vec![SkillCatalogAction::SetState {
            skill_id: skill.id.clone(),
            state: Some(SkillState::Off),
        }],
    };
    let prepared = service.prepare(&off).unwrap();
    service.publish_authorized(&off, &prepared).unwrap();
    let set = super::super::skills_dispatch::prepare_remote_launch_skills(&runtime, &session_id)
        .unwrap()
        .expect("an empty view is still sent so the lease forgets the skill");
    assert!(set.manifest.skills.is_empty());
    assert!(set.packages.is_empty());
    assert_eq!(set.names, None);
}

// MP-08 / MP-10 / MP-11: a failed publication refresh is repaired by the next
// home turn, even when disabling the last skill left nothing to transfer.
#[tokio::test]
async fn leased_home_turn_clears_the_view_when_no_skill_is_effective() {
    let (runtime, service, skill, context, session_id, agent_id) = manual_review_fixture().await;
    let off = SkillCatalogMutation {
        scope: SkillScope::Chariox,
        expected_revision: service.storage.snapshot().unwrap().revision,
        actions: vec![SkillCatalogAction::SetState {
            skill_id: skill.id,
            state: Some(SkillState::Off),
        }],
    };
    let prepared = service.prepare(&off).unwrap();
    let catalog = service.publish_authorized(&off, &prepared).unwrap();
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
    let turn = super::super::skills_dispatch::prepare_remote_turn(
        &runtime,
        &session_id,
        &agent_id,
        prompt.id(),
        &context.leased_agent_id,
        &[],
        &super::super::skills_dispatch::ReportedWorkerRun::Unknown,
    )
    .unwrap()
    .expect("the next turn must repair a missed empty clearing view");
    assert_eq!(turn.set.manifest.catalog_revision, catalog.revision);
    assert!(turn.set.manifest.skills.is_empty());
    assert!(turn.set.packages.is_empty());
    assert_eq!(turn.set.names, None);
}

// MP-08 / MP-10: the home never trusts the worker's name or digest for a typed load.
#[tokio::test]
async fn leased_typed_skill_load_is_recorded_only_for_a_current_effective_skill() {
    let (runtime, service, skill, context, session_id, agent_id) = manual_review_fixture().await;
    let typed = |skill_id: &str, digest: &str| {
        serde_json::json!({
            "delivery": "typed", "prompt_id": "worker-native-prompt",
            "skill_id": skill_id, "digest": digest,
        })
    };
    assert!(
        acknowledge(&runtime, &context, typed(&skill.id, &"b".repeat(64)))
            .await
            .is_err(),
        "a stale digest was recorded"
    );
    assert!(
        acknowledge(&runtime, &context, typed("skill-foreign", &skill.digest))
            .await
            .is_err(),
        "a foreign skill was recorded"
    );
    assert!(service.storage.loads(&session_id).unwrap().is_empty());
    acknowledge(&runtime, &context, typed(&skill.id, &skill.digest))
        .await
        .unwrap();
    let loads = service.storage.loads(&session_id).unwrap();
    assert_eq!(loads.len(), 1);
    assert_eq!(
        (
            &loads[0].skill_id,
            &loads[0].digest,
            &loads[0].agent_id,
            &loads[0].prompt_id
        ),
        (
            &skill.id,
            &skill.digest,
            &agent_id,
            &"worker-native-prompt".to_string()
        )
    );
    assert_eq!(loads[0].reason, SkillLoadReason::Explicit);
    let off = SkillCatalogMutation {
        scope: SkillScope::Chariox,
        expected_revision: service.storage.snapshot().unwrap().revision,
        actions: vec![SkillCatalogAction::SetState {
            skill_id: skill.id.clone(),
            state: Some(SkillState::Off),
        }],
    };
    let prepared = service.prepare(&off).unwrap();
    service.publish_authorized(&off, &prepared).unwrap();
    assert!(
        acknowledge(&runtime, &context, typed(&skill.id, &skill.digest))
            .await
            .is_err(),
        "an Off skill was recorded"
    );
}

// MP-08 / MP-10: a structured Claude run records a skill as Loaded when its hook
// reports the context delivered, once, however often the hook reports it.
#[tokio::test]
async fn claude_hook_delivery_receipt_records_the_local_run_load_once() {
    use crate::skill::service::SkillCatalogService;
    let (runtime, session_id, agent_id, attachment, run) =
        super::tests::local_claude_fixture(crate::DaemonConfig::for_tests()).await;
    let config = runtime.owned.config_projection.snapshot();
    let storage = SkillCatalogStorage::new(
        config.user_config_path.parent().unwrap(),
        runtime.owned.durable_state_store.clone(),
    )
    .unwrap();
    let service = SkillCatalogService::new(storage.clone());
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
    let catalog = service.publish_authorized(&mutation, &prepared).unwrap();
    let skill = catalog.catalogs[0].skills[0].clone();
    let submission = runtime
        .owned
        .submit_local_prepared_prompt(&crate::app::KernelPreparedPromptSubmission {
            session_id: session_id.clone(),
            prompt: crate::session::PromptQueueItem::new(
                "claude-hook-turn",
                &attachment,
                &agent_id,
                "$review inspect source",
                crate::session::PromptStatus::Queued,
            ),
            force_queue: false,
            refresh_projection: true,
        })
        .unwrap()
        .unwrap();
    let prompt = match submission.outcome {
        crate::session::PromptSubmissionOutcome::Started { prompt } => prompt,
        other => panic!("unexpected prompt: {other:?}"),
    };
    let session = runtime
        .owned
        .session_store
        .get_session(&session_id)
        .unwrap();
    let authority = SkillTurnAuthority {
        catalog_revision: catalog.revision,
        project_id: session.project_id().into(),
        provider_run_id: run.id().into(),
        context_generation: "generation".into(),
        explicit_skill_ids: vec![skill.id.clone()],
        body_budget_tokens: 100,
        reserved_body_tokens: Default::default(),
        pending_loads: vec![SkillLoadRecord {
            skill_id: skill.id.clone(),
            digest: skill.digest.clone(),
            session_id: session_id.clone(),
            agent_id: agent_id.clone(),
            run_id: run.id().into(),
            prompt_id: prompt.id().into(),
            reason: SkillLoadReason::Explicit,
            context_generation: "generation".into(),
            delivered_at_ms: 0,
        }],
        prompt_delivery_skill_ids: [skill.id.clone()].into(),
        remote_roots: Default::default(),
        native_tool_loads: Default::default(),
        compaction_revision: None,
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
    let bridge = super::super::skills_dispatch::bridge(&runtime);
    let silent = crate::provider::ProviderPromptSignalBatch::default();
    bridge.acknowledge_tools(&run, &silent).unwrap();
    assert!(
        storage.loads(&session_id).unwrap().is_empty(),
        "a queued stream-json write is not a delivery"
    );
    let delivered = crate::provider::ProviderPromptSignalBatch {
        hook_context_delivered: true,
        ..Default::default()
    };
    bridge.acknowledge_tools(&run, &delivered).unwrap();
    bridge.acknowledge_tools(&run, &delivered).unwrap();
    let loads = storage.loads(&session_id).unwrap();
    assert_eq!(loads.len(), 1, "{loads:?}");
    assert_eq!(loads[0].skill_id, skill.id);
    assert_eq!(loads[0].run_id, "local-claude-run");
}

// A native composer outlives the typed turns that clear its binding's active run id, so a
// published catalog change has to reach every lease, not only those with a running turn.
#[tokio::test]
async fn catalog_changes_refresh_the_view_of_every_lease() {
    let (runtime, _session_id, agent_id, _attachment, _binding) =
        super::tests::mp11_fixture(crate::DaemonConfig::for_tests()).await;
    for active in [Some("worker-run-1".to_string()), None] {
        runtime
            .owned
            .agent_store
            .set_remote_execution_active_worker_provider_run_id(&agent_id, active)
            .unwrap();
        assert_eq!(runtime.refresh_leased_native_skill_views().await, 1);
    }
}

// MP-08 / MP-10 / MP-11: task completion clears the active-turn run binding,
// although a native composer keeps the same process for its next typed task.
#[tokio::test]
async fn leased_typed_skill_receipt_uses_the_live_native_run_between_home_projections() {
    use crate::provider::{
        LaunchProviderRequest, ProviderClientInterface, ProviderLaunchResult, RuntimeProviderRun,
    };
    let (runtime, service, skill, context, session_id, agent_id) = manual_review_fixture().await;
    let request = LaunchProviderRequest::new(&session_id, "claude", "claude", "default", "haiku")
        .with_agent_id(&agent_id)
        .with_client_interface(ProviderClientInterface::NativeTui);
    let mut run = RuntimeProviderRun::new(
        crate::provider::projected_leased_provider_run_id(
            &context.leased_agent_id,
            &context.worker_provider_run_id,
        ),
        &request,
        ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "native-claude-composer".into(),
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
        .set_remote_execution_active_worker_provider_run_id(&agent_id, None)
        .unwrap();
    let typed = serde_json::json!({"delivery":"typed","prompt_id":"next-worker-native-prompt","skill_id":skill.id,"digest":skill.digest});
    let peer = runtime.with_relay_peer_authority(
        crate::runtime::relay_peer_authority::test_peer_authority(&context.worker_kernel_id),
    );
    assert!(
        peer.authorize_forwarded_workspace_context(&context)
            .is_err(),
        "an idle receipt cannot grant ordinary tool access"
    );
    let request = RelayPeerRequest::ForwardCapabilityRuntimeTool {
        context: context.clone(),
        tool_name: "_chariox_skill_delivery_ack".into(),
        arguments: typed.clone(),
    };
    let admitted = peer
        .prepare_forwarded_peer_request(&request)
        .await
        .expect("a current native run still owns typed delivery receipts while the home is idle");
    // The next worker projection can arrive between receipt admission and persistence.
    runtime
        .owned
        .agent_store
        .set_remote_execution_active_worker_provider_run_id(
            &agent_id,
            Some(context.worker_provider_run_id.clone()),
        )
        .unwrap();
    let (result, _) = admitted
        .dispatch_forwarded_capability_runtime_tool_call(
            context.clone(),
            "_chariox_skill_delivery_ack".into(),
            typed.clone(),
        )
        .await
        .unwrap();
    assert!(result.ok);
    assert_eq!(service.storage.loads(&session_id).unwrap().len(), 1);
    runtime
        .owned
        .agent_store
        .set_remote_execution_active_worker_provider_run_id(&agent_id, None)
        .unwrap();
    let mut stale = context.clone();
    stale.worker_provider_run_id = "replaced-native-run".into();
    assert!(acknowledge(&runtime, &stale, typed.clone()).await.is_err());
    let mut foreign = context.clone();
    foreign.leased_agent_id = "replaced-lease".into();
    assert!(acknowledge(&runtime, &foreign, typed.clone())
        .await
        .is_err());
    let mut ended = runtime
        .owned
        .provider_run_projection
        .get_for_agent(&session_id, &agent_id)
        .unwrap();
    ended.mark_ended();
    runtime
        .owned
        .provider_run_projection
        .update_remote_snapshot(ended);
    assert!(
        acknowledge(&runtime, &context, typed).await.is_err(),
        "an ended native run cannot acknowledge new loads"
    );
    assert_eq!(service.storage.loads(&session_id).unwrap().len(), 1);
}

// MP-08 / MP-10 / MP-11: the first home turn after a typed task receives its
// hook ACK before the home can restore the active worker run binding.
#[tokio::test]
async fn leased_home_skill_receipt_uses_the_live_native_run_during_dispatch() {
    use crate::provider::{
        LaunchProviderRequest, ProviderClientInterface, ProviderLaunchResult, RuntimeProviderRun,
    };
    let (runtime, service, _skill, mut context, session_id, agent_id) =
        manual_review_fixture().await;
    let request = LaunchProviderRequest::new(&session_id, "claude", "claude", "default", "haiku")
        .with_agent_id(&agent_id)
        .with_client_interface(ProviderClientInterface::NativeTui);
    let mut run = RuntimeProviderRun::new(
        crate::provider::projected_leased_provider_run_id(
            &context.leased_agent_id,
            &context.worker_provider_run_id,
        ),
        &request,
        ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "native-claude-composer".into(),
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
        .set_remote_execution_active_worker_provider_run_id(&agent_id, None)
        .unwrap();
    let session = runtime
        .owned
        .session_store
        .get_session(&session_id)
        .unwrap();
    let active = runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &agent_id)
        .unwrap();
    let mut dispatching = active.clone();
    dispatching.set_status(crate::session::PromptStatus::Dispatching);
    assert!(runtime
        .owned
        .prompt_state_owner
        .replace_active_prompt_if_matches(&session, &agent_id, &active, dispatching,));
    context.home_prompt_id = Some(active.id().into());
    let peer = runtime.with_relay_peer_authority(
        crate::runtime::relay_peer_authority::test_peer_authority(&context.worker_kernel_id),
    );
    assert!(
        peer.authorize_forwarded_workspace_context(&context)
            .is_err(),
        "a delivery receipt must not admit ordinary home tools before the run is rebound"
    );
    let acknowledgement = serde_json::json!({"delivery":"prompt"});
    let request = RelayPeerRequest::ForwardCapabilityRuntimeTool {
        context: context.clone(),
        tool_name: "_chariox_skill_delivery_ack".into(),
        arguments: acknowledgement.clone(),
    };
    let admitted = peer.prepare_forwarded_peer_request(&request).await.expect(
        "the current home prompt can acknowledge the same live native process before rebinding",
    );
    let (result, _) = admitted
        .dispatch_forwarded_capability_runtime_tool_call(
            context.clone(),
            "_chariox_skill_delivery_ack".into(),
            acknowledgement.clone(),
        )
        .await
        .unwrap();
    assert!(result.ok);
    assert!(service.storage.loads(&session_id).unwrap().is_empty());
    let mut stale = context.clone();
    stale.home_prompt_id = Some("replaced-home-prompt".into());
    assert!(acknowledge(&runtime, &stale, acknowledgement.clone())
        .await
        .is_err());
    stale = context;
    stale.worker_provider_run_id = "replaced-native-run".into();
    assert!(acknowledge(&runtime, &stale, acknowledgement)
        .await
        .is_err());
}
