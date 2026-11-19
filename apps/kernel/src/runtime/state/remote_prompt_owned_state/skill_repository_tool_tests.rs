//! SV-15: an agent imports and updates repository skill packages through
//! chariox.manage_skills, on the same service path the Skills window uses.
use crate::skill::{
    repository_tests::{git, install_all, Repo},
    service::SkillCatalogService,
    storage::SkillCatalogStorage,
};
use crate::transport::runtime_tools::MANAGE_SKILLS_TOOL;

#[tokio::test]
async fn an_agent_resolves_repository_imports_and_updates_through_manage_skills() {
    let (runtime, session_id, agent_id, attachment, run) =
        super::tests::local_claude_fixture(crate::DaemonConfig::for_tests()).await;
    let config = runtime.owned.config_projection.snapshot();
    let storage = SkillCatalogStorage::new(
        config.user_config_path.parent().unwrap(),
        runtime.owned.durable_state_store.clone(),
    )
    .unwrap();
    let service = SkillCatalogService::new(storage);
    let repo = Repo::new();
    let installed = install_all(&repo, &repo.head(), &["skills/alpha", "skills/beta"]);
    let prepared = service.prepare(&installed).unwrap();
    service.publish_authorized(&installed, &prepared).unwrap();
    std::fs::write(
        repo.root.join("skills/alpha/SKILL.md"),
        "---\nname: alpha\ndescription: Changed.\n---\nNew.\n",
    )
    .unwrap();
    git(&repo.root, &["commit", "--quiet", "-am", "second"]);
    let submission = runtime
        .owned
        .submit_local_prepared_prompt(&crate::app::KernelPreparedPromptSubmission {
            session_id: session_id.clone(),
            prompt: crate::session::PromptQueueItem::new(
                "repository-turn",
                &attachment,
                &agent_id,
                "update the skills repository",
                crate::session::PromptStatus::Queued,
            ),
            force_queue: false,
            refresh_projection: true,
        })
        .unwrap()
        .unwrap();
    let crate::session::PromptSubmissionOutcome::Started { prompt } = submission.outcome else {
        panic!("the turn should start");
    };

    let update = runtime
        .dispatch_capability_runtime_tool_call(
            &run,
            MANAGE_SKILLS_TOOL,
            serde_json::json!({"action":"resolve_repository_update","package_id":"package-repo","origin_prompt_id":prompt.id()}),
        )
        .await
        .unwrap();
    assert!(update.ok, "{:?}", update.payload);
    assert_eq!(update.payload["kind"], "repository_update");
    assert_eq!(update.payload["revision"], repo.head());
    assert_eq!(update.payload["changed"][0]["member"]["name"], "alpha");

    // The owner's local repository stays the owner's: the agent's review of
    // the ready update reaches the source boundary, not an owner-only refusal.
    let preview = runtime
        .dispatch_capability_runtime_tool_call(
            &run,
            MANAGE_SKILLS_TOOL,
            serde_json::json!({"action":"preview","mutation":update.payload["mutation"],"origin_prompt_id":prompt.id()}),
        )
        .await;
    let preview = format!("{preview:?}");
    assert!(
        preview.contains("only public https repositories"),
        "{preview}"
    );

    for url in [repo.url(), "https://127.0.0.1/org/skills.git".into()] {
        let resolved = runtime
            .dispatch_capability_runtime_tool_call(
                &run,
                MANAGE_SKILLS_TOOL,
                serde_json::json!({"action":"resolve_repository","url":url,"origin_prompt_id":prompt.id()}),
            )
            .await;
        let resolved = format!("{resolved:?}");
        assert!(
            resolved.contains("only public https repositories"),
            "{resolved}"
        );
    }
    let stale = runtime
        .dispatch_capability_runtime_tool_call(
            &run,
            MANAGE_SKILLS_TOOL,
            serde_json::json!({"action":"resolve_repository_update","package_id":"package-repo","origin_prompt_id":"an-old-turn"}),
        )
        .await;
    assert!(
        format!("{stale:?}").contains("current prompt_id"),
        "{stale:?}"
    );
}

// A leased agent's manage_skills call reaches the home over the forwarded
// capability path; the repository actions travel it like preview and publish.
#[tokio::test]
async fn a_leased_agent_resolves_a_repository_update_on_the_home() {
    let (runtime, context, storage, _) = super::skill_delivery_tests::fixture().await;
    let service = SkillCatalogService::new(storage);
    let repo = Repo::new();
    let installed = install_all(&repo, &repo.head(), &["skills/alpha"]);
    let prepared = service.prepare(&installed).unwrap();
    service.publish_authorized(&installed, &prepared).unwrap();
    let arguments = serde_json::json!({
        "action":"resolve_repository_update",
        "package_id":"package-repo",
        "origin_prompt_id":context.home_prompt_id.clone().unwrap(),
    });
    let request = crate::transport::relay_peer::RelayPeerRequest::ForwardCapabilityRuntimeTool {
        context: context.clone(),
        tool_name: MANAGE_SKILLS_TOOL.into(),
        arguments: arguments.clone(),
    };
    let admitted = runtime
        .with_relay_peer_authority(crate::runtime::relay_peer_authority::test_peer_authority(
            &context.worker_kernel_id,
        ))
        .prepare_forwarded_peer_request(&request)
        .await
        .unwrap();
    let (result, _) = admitted
        .dispatch_forwarded_capability_runtime_tool_call(
            context.clone(),
            MANAGE_SKILLS_TOOL.into(),
            arguments,
        )
        .await
        .unwrap();
    assert!(result.ok, "{:?}", result.payload);
    assert_eq!(result.payload["kind"], "repository_update");
    assert_eq!(result.payload["unchanged"][0]["name"], "alpha");
}
