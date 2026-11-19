//! MP-08 / MP-10 / MP-11: actual dispatcher contract behind the native proxy's
//! foundation-only compatibility path. This is not real-provider acceptance.
use super::*;

#[tokio::test]
async fn native_skills_catalog_service_does_not_block_ordinary_prompt_admission() {
    let worktree = crate::test_support::TestWorktree::new("native-skills-foundation");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            &session_id,
            "native-skills-foundation",
            ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    // Reuse the adjacent router tests' DevStub fixture. The real Codex
    // account gate is outside this provider-independent dispatcher contract.
    launch_test_provider(
        &mut app,
        &session_id,
        &agent_id,
        "dev-stub",
        "claude-code",
        "sonnet",
    );
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
    let request = LocalDaemonRequest::SkillsV2(crate::skill::v2::SkillsV2Request::Catalog {
        project_id: Some(session.project_id().to_string()),
        agent_id: None,
    });
    let command =
        KernelCommand::from_local_request("native-foundation-catalog", None, None, &request);
    let response = router.dispatch(command, request).await.unwrap();
    assert!(matches!(response, LocalDaemonResponse::SkillsV2 { .. }));
    // This serialized request is exactly the ordinary native client's existing
    // prompt path. No v2 submission was attempted or retried.
    for (index, prompt) in ["explain this code\n", "$review inspect source\n"]
        .into_iter()
        .enumerate()
    {
        let request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: Some(agent_id.clone()),
            prompt: prompt.into(),
            attachments: Vec::new(),
        });
        let command = KernelCommand::from_local_request(
            format!("native-foundation-submit-{index}"),
            None,
            None,
            &request,
        );
        let response = router.dispatch(command, request).await.unwrap();
        assert!(matches!(
            response,
            LocalDaemonResponse::PromptSubmitted { .. }
        ));
    }
    assert!(router
        .agent_runtime_projection
        .get(&agent_id)
        .unwrap()
        .active_prompt
        .is_some());
}

// SV-02: a `$name` chip that is stale, Off or replaced is refused at admission, so the user sees
// why instead of a prompt that was accepted and then never ran.
#[tokio::test]
async fn skills_v2_submit_refuses_a_stale_off_or_replaced_selection() {
    use crate::skill::v2::{
        SkillCatalogAction, SkillCatalogMutation, SkillScope, SkillSelection, SkillState,
        SkillsV2Request, SkillsV2Response,
    };
    let worktree = crate::test_support::TestWorktree::new("skills-v2-admission");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            &session_id,
            "skills-v2-admission",
            ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    launch_test_provider(
        &mut app,
        &session_id,
        &agent_id,
        "dev-stub",
        "claude-code",
        "sonnet",
    );
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
    let installed =
        crate::test_support::install_skill_through_requests(&router, "probe-one", 0).await;
    let catalog = router
        .runtime_state()
        .execute_skills_catalog_request(
            SkillsV2Request::Catalog {
                project_id: Some(session.project_id().to_string()),
                agent_id: None,
            },
            crate::session::DEFAULT_LOCAL_USER_ID.to_owned(),
        )
        .await
        .unwrap();
    let LocalDaemonResponse::SkillsV2 {
        result: SkillsV2Response::Catalog { skills, .. },
    } = catalog
    else {
        panic!("Catalog must answer with the skills");
    };
    let skill_id = skills[0].skill.id.clone();
    let submit = |index: usize, selection: SkillSelection| {
        let request = LocalDaemonRequest::SkillsV2(SkillsV2Request::Submit {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: agent_id.clone(),
            prompt: "use the probe\n".into(),
            attachments: Vec::new(),
            selections: vec![selection],
        });
        let command = KernelCommand::from_local_request(
            format!("skills-v2-admission-{index}"),
            None,
            None,
            &request,
        );
        router.dispatch(command, request)
    };
    let selection = |skill_id: &str, name: &str, catalog_revision: u64| SkillSelection::Chariox {
        skill_id: skill_id.into(),
        name: name.into(),
        catalog_revision,
    };
    let stale = submit(0, selection(&skill_id, "probe-one", installed - 1))
        .await
        .unwrap_err()
        .to_string();
    assert!(stale.contains("catalog changed"), "{stale}");

    let off = SkillCatalogMutation {
        scope: SkillScope::Chariox,
        expected_revision: installed,
        actions: vec![SkillCatalogAction::SetState {
            skill_id: skill_id.clone(),
            state: Some(SkillState::Off),
        }],
    };
    let state = router.runtime_state();
    let user = crate::session::DEFAULT_LOCAL_USER_ID.to_owned();
    let LocalDaemonResponse::SkillsV2 {
        result: SkillsV2Response::Preview { review_receipt, .. },
    } = state
        .execute_skills_catalog_request(
            SkillsV2Request::Preview {
                mutation: off.clone(),
            },
            user.clone(),
        )
        .await
        .unwrap()
    else {
        panic!("Preview must answer with a review receipt");
    };
    let LocalDaemonResponse::SkillsV2 {
        result: SkillsV2Response::Published { revision, .. },
    } = state
        .execute_skills_catalog_request(
            SkillsV2Request::Mutate {
                mutation: off,
                review_receipt_id: review_receipt.id,
            },
            user,
        )
        .await
        .unwrap()
    else {
        panic!("Mutate must answer with the published revision");
    };
    let refused = submit(1, selection(&skill_id, "probe-one", revision))
        .await
        .unwrap_err()
        .to_string();
    assert!(refused.contains("is Off"), "{refused}");

    let current =
        crate::test_support::install_skill_through_requests(&router, "probe-two", revision).await;
    let replaced = submit(2, selection(&skill_id, "probe-two", current))
        .await
        .unwrap_err()
        .to_string();
    assert!(replaced.contains("was replaced"), "{replaced}");
    assert!(router
        .agent_runtime_projection
        .get(&agent_id)
        .and_then(|projection| projection.active_prompt)
        .is_none());
}
