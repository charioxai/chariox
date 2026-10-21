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
            &format!("native-foundation-submit-{index}"),
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
