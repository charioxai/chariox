use crate::app::{
    DaemonApp, KernelAgentService, KernelPreparedPromptSubmission, KernelSessionService,
};
use crate::attachment::{AttachRequest, ClientCapabilityLevel};
use crate::session::{PromptQueueItem, PromptStatus, PromptSubmissionOutcome};

#[test]
fn selected_unavailable_account_admits_prompt_on_configured_substitute() {
    let worktree = crate::test_support::TestWorktree::new("substitute-admission");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, agent) = KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let attachment = KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "substitute-admission",
            ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let registry = app.provider_account_profile_registry();
    let primary = registry
        .create_managed(agent.owner_user_id(), "codex", "Selected primary")
        .unwrap();
    registry
        .update_observation(
            agent.owner_user_id(),
            "codex",
            &primary.profile_id,
            crate::account_profile::ProviderAccountAuthState::NotConfigured,
            None,
            None,
            None,
            None,
        )
        .unwrap();
    assert!(matches!(
        registry.require_authenticated(
            agent.owner_user_id(),
            "codex",
            &primary.profile_id,
            Some("primary-model"),
            "test setup"
        ),
        Err(crate::error::DaemonError::ProviderProfileRefused {
            reason: crate::error::ProviderProfileRefusal::Unauthenticated,
            ..
        })
    ));
    app.agents()
        .set_agent_runtime_profile_with_account_profile(
            agent.id(),
            "codex",
            Some("primary-model".into()),
            None,
            Some(primary.profile_id.clone()),
            crate::provider::ProviderResumeState::default(),
        )
        .unwrap();
    app.agents()
        .add_agent_substitute(
            agent.id(),
            crate::agent::AgentSubstituteProfile::new("dev-stub", "usable-substitute", None),
        )
        .unwrap();
    let result = KernelAgentService::new(&mut app).submit_prepared_prompt_for_kernel(
        KernelPreparedPromptSubmission {
            session_id: session.id().to_string(),
            prompt: PromptQueueItem::new(
                "request-with-unavailable-primary",
                attachment.id(),
                agent.id(),
                "Review this change",
                PromptStatus::Queued,
            ),
            force_queue: false,
            refresh_projection: true,
        },
    );
    assert!(result.is_ok(), "a registered unavailable primary with a usable configured substitute must admit the prompt: {:?}", result.as_ref().err());
    let submission = result.unwrap();
    let PromptSubmissionOutcome::Started { prompt } = submission.outcome else {
        panic!("idle prompt must start on its substitute");
    };
    let run = app
        .providers()
        .get_run_for_agent(session.id(), agent.id())
        .unwrap();
    assert_eq!(run.model(), "usable-substitute");
    assert_eq!(run.turn_substitute().unwrap().prompt_id, prompt.id());
    let configured = app.agents().get_agent(agent.id()).unwrap();
    assert_eq!(configured.provider(), "codex");
    assert_eq!(configured.provider_account_profile(), primary.profile_id);
    app.end_agent_provider_run(session.id(), agent.id())
        .unwrap();
}

#[test]
fn substitute_admission_skips_unverified_unauthenticated_and_remote_candidates_in_order() {
    let worktree = crate::test_support::TestWorktree::new("ordered-substitute-admission");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, agent) = KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let attachment = KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "ordered-substitute-admission",
            ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let registry = app.provider_account_profile_registry();
    let mut profiles = Vec::new();
    for (label, state) in [
        (
            "Unavailable primary",
            crate::account_profile::ProviderAccountAuthState::NotConfigured,
        ),
        (
            "Unverified substitute",
            crate::account_profile::ProviderAccountAuthState::Unknown,
        ),
        (
            "Signed-out substitute",
            crate::account_profile::ProviderAccountAuthState::NotConfigured,
        ),
    ] {
        let profile = registry
            .create_managed(agent.owner_user_id(), "codex", label)
            .unwrap();
        registry
            .update_observation(
                agent.owner_user_id(),
                "codex",
                &profile.profile_id,
                state,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        profiles.push(profile);
    }
    app.agents()
        .set_agent_runtime_profile_with_account_profile(
            agent.id(),
            "codex",
            Some("primary-model".into()),
            None,
            Some(profiles[0].profile_id.clone()),
            crate::provider::ProviderResumeState::default(),
        )
        .unwrap();
    for profile in &profiles[1..] {
        app.agents()
            .add_agent_substitute(
                agent.id(),
                crate::agent::AgentSubstituteProfile::new("codex", "skipped-model", None)
                    .with_account_profile(Some(profile.profile_id.clone())),
            )
            .unwrap();
    }
    for candidate in [
        crate::agent::AgentSubstituteProfile::new("dev-stub", "remote", None)
            .with_kernel_id(Some("another-kernel".into())),
        crate::agent::AgentSubstituteProfile::new("dev-stub", "first-usable", None),
        crate::agent::AgentSubstituteProfile::new("dev-stub", "later-usable", None),
    ] {
        app.agents()
            .add_agent_substitute(agent.id(), candidate)
            .unwrap();
    }
    let submission = KernelAgentService::new(&mut app)
        .submit_prepared_prompt_for_kernel(KernelPreparedPromptSubmission {
            session_id: session.id().into(),
            prompt: PromptQueueItem::new(
                "ordered-admission",
                attachment.id(),
                agent.id(),
                "Review this change",
                PromptStatus::Queued,
            ),
            force_queue: false,
            refresh_projection: true,
        })
        .unwrap();
    assert!(matches!(
        submission.outcome,
        PromptSubmissionOutcome::Started { .. }
    ));
    let run = app
        .providers()
        .get_run_for_agent(session.id(), agent.id())
        .unwrap();
    assert_eq!(run.model(), "first-usable");
    assert_eq!(run.turn_substitute().unwrap().tried.len(), 1);
    assert_eq!(
        app.agents()
            .get_agent(agent.id())
            .unwrap()
            .provider_account_profile(),
        profiles[0].profile_id
    );
    app.end_agent_provider_run(session.id(), agent.id())
        .unwrap();
}
