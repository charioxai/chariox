//! MP-08 / MP-10 / MP-11: persist only the current run's confirmed native identity.
use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::provider::{ProviderPromptSignalBatch, RuntimeProviderRun};

pub(super) fn persist_resolved_resume_state(
    app: &mut DaemonApp,
    provider_run: &RuntimeProviderRun,
    poll_result: &ProviderPromptSignalBatch,
) -> Result<(), DaemonError> {
    let Some(resume_state) = poll_result.resolved_resume_state.as_ref() else {
        return Ok(());
    };
    // A substitute run reruns one turn; it never becomes the agent's profile.
    let Some(agent_id) = provider_run
        .agent_instance_id()
        .filter(|_| provider_run.turn_substitute().is_none())
    else {
        return Ok(());
    };
    // A delayed native hook cannot replace a newer run's resume identity.
    if app
        .providers
        .get_run_for_agent(provider_run.session_id(), agent_id)
        .is_none_or(|current| current.id() != provider_run.id())
    {
        return Ok(());
    }
    let durable_state_store = app.durable_state_store();
    app.agents.set_agent_runtime_profile_durably(
        &durable_state_store,
        agent_id,
        provider_run.provider(),
        Some(provider_run.model().to_string()),
        provider_run.variant().map(str::to_string),
        None,
        resume_state.clone(),
        Some(provider_run.id()),
        None,
    )?;
    let _ = crate::app::KernelSessionReadService::new(app)
        .session_snapshot(provider_run.session_id())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn obsolete_run_metadata_cannot_replace_current_native_resume() {
        let root = crate::test_support::TestWorktree::new("native-resume-fence");
        let mut app = crate::test_support::bootstrap_authenticated_app(
            crate::config::DaemonConfig::for_tests(),
        )
        .unwrap();
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(root.session_request())
            .unwrap();
        let request = crate::provider::LaunchProviderRequest::new(
            session.id(),
            "claude",
            "claude-headless",
            "default",
            "haiku",
        )
        .with_agent_id(agent.id());
        let make_run = |id: &str| {
            let mut run = RuntimeProviderRun::new(
                id,
                &request,
                crate::provider::ProviderLaunchResult {
                    endpoint_mode: crate::provider::AgentEndpointMode::Managed,
                    process_label: "metadata-only-native-resume-test".into(),
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
            run
        };
        let mut old = make_run("obsolete-native-run");
        old.mark_ended();
        app.providers_mut().insert_run_for_test(old.clone());
        app.providers_mut()
            .insert_run_for_test(make_run("current-native-run"));
        app.agents_mut()
            .set_agent_runtime_profile_with_account_profile(
                agent.id(),
                "claude-headless",
                Some("haiku".into()),
                None,
                Some("default".into()),
                crate::provider::ProviderResumeState::from_claude_session_id(
                    "current-native-session",
                ),
            )
            .unwrap();
        assert_eq!(
            app.providers()
                .get_run_for_agent(session.id(), agent.id())
                .unwrap()
                .id(),
            "current-native-run",
            "fixture must select the replacement run before testing stale metadata",
        );
        persist_resolved_resume_state(
            &mut app,
            &old,
            &ProviderPromptSignalBatch {
                resolved_resume_state: Some(
                    crate::provider::ProviderResumeState::from_claude_session_id(
                        "obsolete-native-session",
                    ),
                ),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            app.agents()
                .get_agent(agent.id())
                .unwrap()
                .provider_resume_state()
                .claude_session_id(),
            Some("current-native-session")
        );
    }
}
